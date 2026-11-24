//! Android JNI bindings for `com.softn.android.SoftnNative`.
//!
//! All calls are wrapped in `std::panic::catch_unwind` so a Rust panic never
//! crashes the Android ART process without logging.

#![cfg(target_os = "android")]

use std::path::PathBuf;
use std::sync::Mutex;

use jni::objects::{JByteArray, JByteBuffer, JClass, JString};
use jni::sys::{jboolean, jfloat, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use serde_json::json;

use crate::engine::{HostDeviceInfo, InteractionResult, SoftnMobileEngine};

struct EngineHandle {
    engine: Mutex<SoftnMobileEngine>,
}

fn init_android_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("SoftN"),
        );
    });
}

fn jstr(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|js| js.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

fn result_to_json(res: &InteractionResult) -> String {
    serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeCreate(
    mut env: JNIEnv,
    _class: JClass,
    bundle_bytes: JByteArray,
    data_dir: JString,
    width: jint,
    height: jint,
    density: jfloat,
    device_info_json: JString,
) -> jlong {
    init_android_logging();

    let Ok(bytes) = env.convert_byte_array(&bundle_bytes) else {
        log::error!("nativeCreate: failed to read bundle bytes");
        return 0;
    };
    let Ok(data_dir_str) = env.get_string(&data_dir) else {
        log::error!("nativeCreate: failed to read dataDir");
        return 0;
    };
    let data_dir_path = PathBuf::from(data_dir_str.to_string_lossy().into_owned());

    let host_info: HostDeviceInfo = env
        .get_string(&device_info_json)
        .ok()
        .and_then(|s| serde_json::from_str(&s.to_string_lossy()).ok())
        .unwrap_or_default();

    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        SoftnMobileEngine::load_bundle(
            &bytes,
            &data_dir_path,
            width.max(1) as u32,
            height.max(1) as u32,
            density.max(1.0),
            host_info,
        )
    }));

    match res {
        Ok(Ok(engine)) => {
            log::info!("SoftN Mobile Engine booted: {}", engine.bundle_info_json());
            let boxed = Box::new(EngineHandle {
                engine: Mutex::new(engine),
            });
            Box::into_raw(boxed) as jlong
        }
        Ok(Err(err)) => {
            log::error!("SoftN Mobile Engine boot failed: {err}");
            0
        }
        Err(_) => {
            log::error!("SoftN Mobile Engine panicked during nativeCreate");
            0
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeResize(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    width: jint,
    height: jint,
    density: jfloat,
) {
    if handle == 0 {
        return;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            eng.resize(width.max(1) as u32, height.max(1) as u32, density.max(1.0));
        }
    }));
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeRender(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
    rgba_buffer: JByteBuffer,
) -> jboolean {
    if handle == 0 {
        return JNI_FALSE;
    }
    let Ok(ptr) = env.get_direct_buffer_address(&rgba_buffer) else {
        log::error!("nativeRender: buffer is not a direct ByteBuffer");
        return JNI_FALSE;
    };
    let Ok(cap) = env.get_direct_buffer_capacity(&rgba_buffer) else {
        return JNI_FALSE;
    };
    let slice = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };

    let ok = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            eng.render_rgba(slice)
        } else {
            log::error!("nativeRender: engine mutex poisoned");
            false
        }
    })) {
        Ok(v) => v,
        Err(p) => {
            let msg = p
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| p.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            log::error!("nativeRender panicked: {msg}");
            false
        }
    };

    if ok {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeTap(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    x: jfloat,
    y: jfloat,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.on_tap(x, y))
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeScroll(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    delta_y: jfloat,
) -> jboolean {
    if handle == 0 {
        return JNI_FALSE;
    }
    let moved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            eng.on_scroll(delta_y)
        } else {
            false
        }
    }))
    .unwrap_or(false);
    if moved {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativePointerDown(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
    x: jfloat,
    y: jfloat,
) -> jboolean {
    if handle == 0 {
        return JNI_FALSE;
    }
    let captured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            eng.on_pointer_down(x, y)
        } else {
            false
        }
    }))
    .unwrap_or(false);
    if captured {
        JNI_TRUE
    } else {
        JNI_FALSE
    }
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativePointerMove(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    x: jfloat,
    y: jfloat,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.on_pointer_move(x, y))
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativePointerUp(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    x: jfloat,
    y: jfloat,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.on_pointer_up(x, y))
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeSelectOption(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    flid: jint,
    index: jint,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.on_select_option(flid as u32, index))
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeTextInput(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    text: JString,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let txt = env
        .get_string(&text)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.on_text_input(&txt))
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeSubmitIme(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.on_submit_ime())
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeResolveDeviceCall(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
    request_id: jlong,
    ok: jboolean,
    payload_json: JString,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let payload = env
        .get_string(&payload_json)
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.resolve_device_call(request_id as u64, ok == JNI_TRUE, &payload))
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeDrainPendingCalls(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "[]");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(eng) = h.engine.lock() {
            serde_json::to_string(&eng.drain_pending_calls()).unwrap_or_else(|_| "[]".to_string())
        } else {
            "[]".to_string()
        }
    }))
    .unwrap_or_else(|_| "[]".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeTick(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, "{}");
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            result_to_json(&eng.tick())
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeGetBundleInfo(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, &json!({}).to_string());
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(eng) = h.engine.lock() {
            eng.bundle_info_json()
        } else {
            "{}".to_string()
        }
    }))
    .unwrap_or_else(|_| "{}".to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeClearCache(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, r#"{"ok":false}"#);
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            eng.clear_cache()
        } else {
            r#"{"ok":false}"#.to_string()
        }
    }))
    .unwrap_or_else(|_| r#"{"ok":false}"#.to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeClearSiteData(
    mut env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    if handle == 0 {
        return jstr(&mut env, r#"{"ok":false}"#);
    }
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let h = unsafe { &*(handle as *const EngineHandle) };
        if let Ok(mut eng) = h.engine.lock() {
            eng.clear_site_data()
        } else {
            r#"{"ok":false}"#.to_string()
        }
    }))
    .unwrap_or_else(|_| r#"{"ok":false}"#.to_string());
    jstr(&mut env, &out)
}

#[no_mangle]
pub extern "system" fn Java_com_softn_android_SoftnNative_nativeDestroy(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(handle as *mut EngineHandle));
    }));
}
