//! WebGPU (`navigator.gpu`, `canvas.getContext("webgpu")`) on wgpu.
//!
//! The page side is `webgpu_shim.js`: calls recorded into batches of
//! `[op, ...args]`, descriptors as JSON with GPU objects as `{"$": id}`, bytes
//! beside the batch in `__zippHostBytes`. A GPU thread replays them on one wgpu
//! device. wgpu implements WebGPU itself, so most calls map one for one; a
//! render or compute pass is recorded on the page and replayed here when it
//! ends. A canvas context renders into a texture this side owns; at the end of
//! each animation frame it is copied out and becomes the canvas's picture.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use serde_json::Value as Json;
use zipp_vm::embed::HostCtx;

use crate::canvas::{Frame, push_frame};

struct Batch {
    cmds: String,
    payload: Vec<u8>,
    reply: Sender<Reply>,
}

#[derive(Default)]
struct Reply {
    result: String,
    errors: Vec<String>,
    bytes: Option<Vec<u8>>,
    frames: Vec<Frame>,
}

thread_local! {
    static GPU_THREAD: RefCell<Option<Sender<Batch>>> = const { RefCell::new(None) };
}

/// `__zippHostCall("gpu", batch, payloadLength)`.
pub(crate) fn host_call(ctx: &mut dyn HostCtx, args: &[String]) -> Result<String, String> {
    let cmds = args.first().cloned().unwrap_or_default();
    let len: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(0);
    let payload = if len > 0 {
        let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
        if kind != 1 || count < len {
            return Err("Error: gpu: __zippHostBytes is not the batch's bytes".into());
        }
        // SAFETY: the engine pinned `count` bytes at `addr` for this call.
        unsafe { std::slice::from_raw_parts(addr as *const u8, len) }.to_vec()
    } else {
        Vec::new()
    };
    let sender = GPU_THREAD.with(|t| {
        t.borrow_mut()
            .get_or_insert_with(|| {
                let (tx, rx) = mpsc::channel();
                std::thread::Builder::new()
                    .name("zipp-webgpu".into())
                    .spawn(move || gpu_thread(rx))
                    .expect("WebGPU thread");
                tx
            })
            .clone()
    });
    let (reply_tx, reply_rx) = mpsc::channel();
    sender
        .send(Batch { cmds, payload, reply: reply_tx })
        .map_err(|_| "Error: WebGPU is unavailable".to_string())?;
    // Bounded, so that a GPU that never answers cannot freeze the page.
    let reply = reply_rx
        .recv_timeout(Duration::from_secs(30))
        .map_err(|_| "Error: WebGPU did not answer".to_string())?;
    for error in &reply.errors {
        eprintln!("[WEBGPU] {error}");
    }
    if let Some(bytes) = reply.bytes {
        let (addr, count, kind) = ctx.typed_array_region("__zippHostBytes")?;
        if kind == 1 {
            // SAFETY: as above; never more than the array holds.
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), addr as *mut u8, bytes.len().min(count)) };
        }
    }
    for frame in reply.frames {
        push_frame(frame);
    }
    Ok(reply.result)
}

fn gpu_thread(batches: Receiver<Batch>) {
    let mut gpu = Gpu::default();
    for batch in batches {
        let reply = gpu.run(&batch.cmds, &batch.payload);
        let _ = batch.reply.send(reply);
    }
}

/// A pass as the page recorded it, replayed when it ends.
struct Pass {
    encoder: u32,
    compute: bool,
    desc: Json,
    cmds: Vec<Json>,
}

/// A canvas's context: the texture it renders into this frame.
struct Canvas {
    flid: u32,
    format: wgpu::TextureFormat,
    texture: Option<(wgpu::Texture, u32, u32)>,
}

#[derive(Default)]
struct Gpu {
    device: Option<(wgpu::Device, wgpu::Queue)>,
    adapter_info: Option<String>,
    buffers: HashMap<u32, wgpu::Buffer>,
    mapped: HashSet<u32>,
    textures: HashMap<u32, wgpu::Texture>,
    views: HashMap<u32, wgpu::TextureView>,
    samplers: HashMap<u32, wgpu::Sampler>,
    modules: HashMap<u32, wgpu::ShaderModule>,
    bind_group_layouts: HashMap<u32, wgpu::BindGroupLayout>,
    pipeline_layouts: HashMap<u32, wgpu::PipelineLayout>,
    bind_groups: HashMap<u32, wgpu::BindGroup>,
    render_pipelines: HashMap<u32, wgpu::RenderPipeline>,
    compute_pipelines: HashMap<u32, wgpu::ComputePipeline>,
    encoders: HashMap<u32, wgpu::CommandEncoder>,
    passes: HashMap<u32, Pass>,
    command_buffers: HashMap<u32, wgpu::CommandBuffer>,
    canvases: HashMap<u32, Canvas>,
}

// ---- Reading the page's JSON ------------------------------------------------

fn num(v: Option<&Json>) -> Option<f64> {
    v.and_then(Json::as_f64)
}
fn u32_of(v: Option<&Json>, default: u32) -> u32 {
    num(v).map(|n| n as u32).unwrap_or(default)
}
fn u64_of(v: Option<&Json>, default: u64) -> u64 {
    num(v).map(|n| n as u64).unwrap_or(default)
}
fn str_of(v: Option<&Json>) -> Option<&str> {
    v.and_then(Json::as_str)
}
/// The number of the GPU object a descriptor names (`{"$": id}`).
fn obj(v: Option<&Json>) -> Option<u32> {
    v.and_then(|v| v.get("$")).and_then(Json::as_f64).map(|n| n as u32)
}

fn texture_format(name: &str) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as F;
    Some(match name {
        "r8unorm" => F::R8Unorm,
        "r8snorm" => F::R8Snorm,
        "r8uint" => F::R8Uint,
        "r8sint" => F::R8Sint,
        "r16uint" => F::R16Uint,
        "r16sint" => F::R16Sint,
        "r16float" => F::R16Float,
        "rg8unorm" => F::Rg8Unorm,
        "r32uint" => F::R32Uint,
        "r32sint" => F::R32Sint,
        "r32float" => F::R32Float,
        "rg16float" => F::Rg16Float,
        "rgba8unorm" => F::Rgba8Unorm,
        "rgba8unorm-srgb" => F::Rgba8UnormSrgb,
        "rgba8snorm" => F::Rgba8Snorm,
        "rgba8uint" => F::Rgba8Uint,
        "rgba8sint" => F::Rgba8Sint,
        "bgra8unorm" => F::Bgra8Unorm,
        "bgra8unorm-srgb" => F::Bgra8UnormSrgb,
        "rgb10a2unorm" => F::Rgb10a2Unorm,
        "rg32float" => F::Rg32Float,
        "rg32uint" => F::Rg32Uint,
        "rgba16float" => F::Rgba16Float,
        "rgba32float" => F::Rgba32Float,
        "rgba32uint" => F::Rgba32Uint,
        "depth16unorm" => F::Depth16Unorm,
        "depth24plus" => F::Depth24Plus,
        "depth24plus-stencil8" => F::Depth24PlusStencil8,
        "depth32float" => F::Depth32Float,
        "stencil8" => F::Stencil8,
        _ => return None,
    })
}

fn vertex_format(name: &str) -> Option<wgpu::VertexFormat> {
    use wgpu::VertexFormat as V;
    Some(match name {
        "uint8x2" => V::Uint8x2,
        "uint8x4" => V::Uint8x4,
        "sint8x2" => V::Sint8x2,
        "sint8x4" => V::Sint8x4,
        "unorm8x2" => V::Unorm8x2,
        "unorm8x4" => V::Unorm8x4,
        "snorm8x2" => V::Snorm8x2,
        "snorm8x4" => V::Snorm8x4,
        "uint16x2" => V::Uint16x2,
        "uint16x4" => V::Uint16x4,
        "sint16x2" => V::Sint16x2,
        "sint16x4" => V::Sint16x4,
        "unorm16x2" => V::Unorm16x2,
        "unorm16x4" => V::Unorm16x4,
        "float16x2" => V::Float16x2,
        "float16x4" => V::Float16x4,
        "float32" => V::Float32,
        "float32x2" => V::Float32x2,
        "float32x3" => V::Float32x3,
        "float32x4" => V::Float32x4,
        "uint32" => V::Uint32,
        "uint32x2" => V::Uint32x2,
        "uint32x3" => V::Uint32x3,
        "uint32x4" => V::Uint32x4,
        "sint32" => V::Sint32,
        "sint32x2" => V::Sint32x2,
        "sint32x3" => V::Sint32x3,
        "sint32x4" => V::Sint32x4,
        _ => return None,
    })
}

fn compare(name: Option<&str>) -> Option<wgpu::CompareFunction> {
    use wgpu::CompareFunction as C;
    Some(match name? {
        "never" => C::Never,
        "less" => C::Less,
        "equal" => C::Equal,
        "less-equal" => C::LessEqual,
        "greater" => C::Greater,
        "not-equal" => C::NotEqual,
        "greater-equal" => C::GreaterEqual,
        "always" => C::Always,
        _ => return None,
    })
}

fn blend_component(v: Option<&Json>) -> wgpu::BlendComponent {
    let factor = |name: Option<&str>, default| {
        use wgpu::BlendFactor as B;
        match name {
            Some("zero") => B::Zero,
            Some("one") => B::One,
            Some("src") => B::Src,
            Some("one-minus-src") => B::OneMinusSrc,
            Some("src-alpha") => B::SrcAlpha,
            Some("one-minus-src-alpha") => B::OneMinusSrcAlpha,
            Some("dst") => B::Dst,
            Some("one-minus-dst") => B::OneMinusDst,
            Some("dst-alpha") => B::DstAlpha,
            Some("one-minus-dst-alpha") => B::OneMinusDstAlpha,
            Some("src-alpha-saturated") => B::SrcAlphaSaturated,
            Some("constant") => B::Constant,
            Some("one-minus-constant") => B::OneMinusConstant,
            _ => default,
        }
    };
    let operation = match v.and_then(|v| str_of(v.get("operation"))) {
        Some("subtract") => wgpu::BlendOperation::Subtract,
        Some("reverse-subtract") => wgpu::BlendOperation::ReverseSubtract,
        Some("min") => wgpu::BlendOperation::Min,
        Some("max") => wgpu::BlendOperation::Max,
        _ => wgpu::BlendOperation::Add,
    };
    wgpu::BlendComponent {
        src_factor: factor(v.and_then(|v| str_of(v.get("srcFactor"))), wgpu::BlendFactor::One),
        dst_factor: factor(v.and_then(|v| str_of(v.get("dstFactor"))), wgpu::BlendFactor::Zero),
        operation,
    }
}

fn extent(v: Option<&Json>) -> wgpu::Extent3d {
    match v {
        Some(Json::Array(a)) => wgpu::Extent3d {
            width: u32_of(a.first(), 1),
            height: u32_of(a.get(1), 1),
            depth_or_array_layers: u32_of(a.get(2), 1),
        },
        Some(o) => wgpu::Extent3d {
            width: u32_of(o.get("width"), 1),
            height: u32_of(o.get("height"), 1),
            depth_or_array_layers: u32_of(o.get("depthOrArrayLayers"), 1),
        },
        None => wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    }
}

fn origin(v: Option<&Json>) -> wgpu::Origin3d {
    match v {
        Some(Json::Array(a)) => wgpu::Origin3d { x: u32_of(a.first(), 0), y: u32_of(a.get(1), 0), z: u32_of(a.get(2), 0) },
        Some(o) => wgpu::Origin3d { x: u32_of(o.get("x"), 0), y: u32_of(o.get("y"), 0), z: u32_of(o.get("z"), 0) },
        None => wgpu::Origin3d::ZERO,
    }
}

fn color(v: Option<&Json>) -> wgpu::Color {
    match v {
        Some(Json::Array(a)) => wgpu::Color {
            r: num(a.first()).unwrap_or(0.0),
            g: num(a.get(1)).unwrap_or(0.0),
            b: num(a.get(2)).unwrap_or(0.0),
            a: num(a.get(3)).unwrap_or(0.0),
        },
        Some(o) => wgpu::Color {
            r: num(o.get("r")).unwrap_or(0.0),
            g: num(o.get("g")).unwrap_or(0.0),
            b: num(o.get("b")).unwrap_or(0.0),
            a: num(o.get("a")).unwrap_or(0.0),
        },
        None => wgpu::Color::TRANSPARENT,
    }
}

fn view_dimension(name: Option<&str>) -> Option<wgpu::TextureViewDimension> {
    use wgpu::TextureViewDimension as D;
    Some(match name? {
        "1d" => D::D1,
        "2d" => D::D2,
        "2d-array" => D::D2Array,
        "cube" => D::Cube,
        "cube-array" => D::CubeArray,
        "3d" => D::D3,
        _ => return None,
    })
}

fn address_mode(name: Option<&str>) -> wgpu::AddressMode {
    match name {
        Some("repeat") => wgpu::AddressMode::Repeat,
        Some("mirror-repeat") => wgpu::AddressMode::MirrorRepeat,
        _ => wgpu::AddressMode::ClampToEdge,
    }
}

fn filter(name: Option<&str>) -> wgpu::FilterMode {
    if name == Some("linear") { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest }
}

impl Gpu {
    fn device(&self) -> Result<&(wgpu::Device, wgpu::Queue), String> {
        self.device.as_ref().ok_or_else(|| "no device: call requestAdapter first".to_string())
    }

    fn request_adapter(&mut self) -> Result<String, String> {
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.flags = wgpu::InstanceFlags::empty();
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
            apply_limit_buckets: false,
        }))
        .map_err(|e| e.to_string())?;
        let info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("zipp-webgpu"),
            ..Default::default()
        }))
        .map_err(|e| e.to_string())?;
        device.on_uncaptured_error(Arc::new(|e| eprintln!("[WEBGPU] {e}")));
        self.device = Some((device, queue));
        let info = serde_json::json!({ "name": info.name, "vendor": format!("{:?}", info.backend) }).to_string();
        self.adapter_info = Some(info.clone());
        Ok(info)
    }

    fn run(&mut self, cmds: &str, payload: &[u8]) -> Reply {
        let mut reply = Reply::default();
        let cmds: Vec<Json> = match serde_json::from_str(cmds) {
            Ok(Json::Array(c)) => c,
            _ => {
                reply.errors.push("malformed batch".into());
                return reply;
            }
        };
        for cmd in &cmds {
            let op = str_of(cmd.get(0)).unwrap_or("").to_string();
            match self.exec(&op, cmd, payload, &mut reply) {
                Ok(result) => reply.result = result,
                Err(e) => {
                    reply.result = String::new();
                    reply.errors.push(format!("{op}: {e}"));
                }
            }
        }
        reply
    }

    fn bytes(payload: &[u8], at: u64, len: u64) -> Result<&[u8], String> {
        payload.get(at as usize..(at + len) as usize).ok_or_else(|| "bytes out of range".to_string())
    }

    fn exec(&mut self, op: &str, cmd: &Json, payload: &[u8], reply: &mut Reply) -> Result<String, String> {
        let a = |i: usize| cmd.get(i);
        let id = |i: usize| u32_of(cmd.get(i), 0);
        if op == "requestAdapter" {
            if let Some(info) = &self.adapter_info {
                return Ok(info.clone());
            }
            return match self.request_adapter() {
                Ok(info) => Ok(info),
                Err(e) => {
                    reply.errors.push(format!("no adapter: {e}"));
                    Ok("null".into())
                }
            };
        }
        let (device, queue) = {
            let (d, q) = self.device()?;
            (d.clone(), q.clone())
        };
        match op {
            "createBuffer" => {
                // Mapped-at-creation buffers must be a multiple of 4 bytes.
                let size = (u64_of(a(2), 0) + 3) & !3;
                let usage = wgpu::BufferUsages::from_bits_truncate(u32_of(a(3), 0));
                let mapped = a(4).and_then(Json::as_bool).unwrap_or(false);
                let buffer = device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage, mapped_at_creation: mapped });
                if mapped {
                    self.mapped.insert(id(1));
                }
                self.buffers.insert(id(1), buffer);
            }
            "writeBuffer" => {
                let buffer = self.buffers.get(&id(1)).ok_or("no such buffer")?;
                queue.write_buffer(buffer, u64_of(a(2), 0), Self::bytes(payload, u64_of(a(3), 0), u64_of(a(4), 0))?);
            }
            "writeMapped" => {
                let key = id(1);
                let buffer = self.buffers.get(&key).ok_or("no such buffer")?.clone();
                if !self.mapped.contains(&key) {
                    buffer.slice(..).map_async(wgpu::MapMode::Write, |_| {});
                    let _ = device.poll(wgpu::PollType::wait_indefinitely());
                    self.mapped.insert(key);
                }
                let data = Self::bytes(payload, u64_of(a(3), 0), u64_of(a(4), 0))?;
                let offset = u64_of(a(2), 0) as usize;
                let mut range = buffer.slice(..).get_mapped_range_mut().map_err(|e| e.to_string())?;
                let end = (offset + data.len()).min(range.len());
                range.slice(offset..end).copy_from_slice(&data[..end - offset]);
            }
            "unmap" => {
                if self.mapped.remove(&id(1))
                    && let Some(buffer) = self.buffers.get(&id(1))
                {
                    buffer.unmap();
                }
            }
            "mapRead" => {
                let buffer = self.buffers.get(&id(1)).ok_or("no such buffer")?.clone();
                let (offset, size) = (u64_of(a(2), 0), u64_of(a(3), 0));
                let slice = buffer.slice(offset..offset + size);
                slice.map_async(wgpu::MapMode::Read, |_| {});
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                reply.bytes = Some(slice.get_mapped_range().map_err(|e| e.to_string())?.to_vec());
                buffer.unmap();
            }
            "destroy" => {
                if let Some(b) = self.buffers.remove(&id(1)) {
                    b.destroy();
                }
                if let Some(t) = self.textures.remove(&id(1)) {
                    t.destroy();
                }
            }
            "createTexture" => {
                let desc = a(2).ok_or("no descriptor")?;
                let format = str_of(desc.get("format")).and_then(texture_format).ok_or("unsupported texture format")?;
                let dimension = match str_of(desc.get("dimension")) {
                    Some("1d") => wgpu::TextureDimension::D1,
                    Some("3d") => wgpu::TextureDimension::D3,
                    _ => wgpu::TextureDimension::D2,
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: extent(desc.get("size")),
                    mip_level_count: u32_of(desc.get("mipLevelCount"), 1),
                    sample_count: u32_of(desc.get("sampleCount"), 1),
                    dimension,
                    format,
                    usage: wgpu::TextureUsages::from_bits_truncate(u32_of(desc.get("usage"), 0)),
                    view_formats: &[],
                });
                self.textures.insert(id(1), texture);
            }
            "createView" => {
                let texture = self.textures.get(&id(1)).ok_or("no such texture")?;
                let desc = a(3);
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: desc.and_then(|d| str_of(d.get("format"))).and_then(texture_format),
                    dimension: view_dimension(desc.and_then(|d| str_of(d.get("dimension")))),
                    base_mip_level: u32_of(desc.and_then(|d| d.get("baseMipLevel")), 0),
                    mip_level_count: desc.and_then(|d| num(d.get("mipLevelCount"))).map(|n| n as u32),
                    base_array_layer: u32_of(desc.and_then(|d| d.get("baseArrayLayer")), 0),
                    array_layer_count: desc.and_then(|d| num(d.get("arrayLayerCount"))).map(|n| n as u32),
                    ..Default::default()
                });
                self.views.insert(id(2), view);
            }
            "createSampler" => {
                let d = a(2);
                let get = |k: &str| d.and_then(|d| str_of(d.get(k)));
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    address_mode_u: address_mode(get("addressModeU")),
                    address_mode_v: address_mode(get("addressModeV")),
                    address_mode_w: address_mode(get("addressModeW")),
                    mag_filter: filter(get("magFilter")),
                    min_filter: filter(get("minFilter")),
                    compare: compare(get("compare")),
                    ..Default::default()
                });
                self.samplers.insert(id(1), sampler);
            }
            "createShaderModule" => {
                let code = str_of(a(2)).unwrap_or("").to_string();
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: None,
                    source: wgpu::ShaderSource::Wgsl(Cow::Owned(code)),
                });
                self.modules.insert(id(1), module);
            }
            "createBindGroupLayout" => {
                let entries: Vec<wgpu::BindGroupLayoutEntry> = a(2)
                    .and_then(|d| d.get("entries"))
                    .and_then(Json::as_array)
                    .map(|list| list.iter().filter_map(bind_group_layout_entry).collect())
                    .unwrap_or_default();
                let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &entries });
                self.bind_group_layouts.insert(id(1), layout);
            }
            "createPipelineLayout" => {
                let ids: Vec<Option<u32>> = a(2)
                    .and_then(|d| d.get("bindGroupLayouts"))
                    .and_then(Json::as_array)
                    .map(|l| l.iter().map(|v| obj(Some(v))).collect())
                    .unwrap_or_default();
                let layouts: Vec<Option<&wgpu::BindGroupLayout>> =
                    ids.iter().map(|i| i.and_then(|i| self.bind_group_layouts.get(&i))).collect();
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: None,
                    bind_group_layouts: &layouts,
                    immediate_size: 0,
                });
                self.pipeline_layouts.insert(id(1), layout);
            }
            "createBindGroup" => {
                let desc = a(2).ok_or("no descriptor")?;
                let layout = obj(desc.get("layout")).and_then(|l| self.bind_group_layouts.get(&l)).ok_or("no such layout")?;
                let mut entries = Vec::new();
                for entry in desc.get("entries").and_then(Json::as_array).into_iter().flatten() {
                    let binding = u32_of(entry.get("binding"), 0);
                    let resource = entry.get("resource");
                    let resource = if let Some(r) = obj(resource) {
                        if let Some(s) = self.samplers.get(&r) {
                            wgpu::BindingResource::Sampler(s)
                        } else if let Some(v) = self.views.get(&r) {
                            wgpu::BindingResource::TextureView(v)
                        } else if let Some(b) = self.buffers.get(&r) {
                            b.as_entire_binding()
                        } else {
                            return Err(format!("binding {binding}: no such resource"));
                        }
                    } else {
                        let r = resource.ok_or("no resource")?;
                        let b = obj(r.get("buffer")).and_then(|b| self.buffers.get(&b)).ok_or("no such buffer")?;
                        wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: b,
                            offset: u64_of(r.get("offset"), 0),
                            size: num(r.get("size")).and_then(|s| NonZeroU64::new(s as u64)),
                        })
                    };
                    entries.push(wgpu::BindGroupEntry { binding, resource });
                }
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout, entries: &entries });
                self.bind_groups.insert(id(1), group);
            }
            "createRenderPipeline" => {
                let pipeline = self.render_pipeline(&device, a(2).ok_or("no descriptor")?)?;
                self.render_pipelines.insert(id(1), pipeline);
            }
            "createComputePipeline" => {
                let desc = a(2).ok_or("no descriptor")?;
                let compute = desc.get("compute").ok_or("no compute stage")?;
                let module = obj(compute.get("module")).and_then(|m| self.modules.get(&m)).ok_or("no such module")?;
                let layout = obj(desc.get("layout")).and_then(|l| self.pipeline_layouts.get(&l));
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: None,
                    layout,
                    module,
                    entry_point: str_of(compute.get("entryPoint")),
                    compilation_options: Default::default(),
                    cache: None,
                });
                self.compute_pipelines.insert(id(1), pipeline);
            }
            "getBindGroupLayout" => {
                let index = id(2);
                let layout = if let Some(p) = self.render_pipelines.get(&id(1)) {
                    p.get_bind_group_layout(index)
                } else if let Some(p) = self.compute_pipelines.get(&id(1)) {
                    p.get_bind_group_layout(index)
                } else {
                    return Err("no such pipeline".into());
                };
                self.bind_group_layouts.insert(id(3), layout);
            }
            "createCommandEncoder" => {
                let encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                self.encoders.insert(id(1), encoder);
            }
            "beginRenderPass" | "beginComputePass" => {
                self.passes.insert(id(2), Pass {
                    encoder: id(1),
                    compute: op == "beginComputePass",
                    desc: a(3).cloned().unwrap_or(Json::Null),
                    cmds: Vec::new(),
                });
            }
            "pass" => {
                if let Some(pass) = self.passes.get_mut(&id(1)) {
                    pass.cmds.push(cmd.clone());
                }
            }
            "endPass" => {
                let pass = self.passes.remove(&id(1)).ok_or("no such pass")?;
                self.replay_pass(pass)?;
            }
            "copyBufferToBuffer" => {
                let src = self.buffers.get(&id(2)).ok_or("no such buffer")?;
                let dst = self.buffers.get(&id(4)).ok_or("no such buffer")?;
                let encoder = self.encoders.get_mut(&id(1)).ok_or("no such encoder")?;
                encoder.copy_buffer_to_buffer(src, u64_of(a(3), 0), dst, u64_of(a(5), 0), u64_of(a(6), 0));
            }
            "clearBuffer" => {
                let buffer = self.buffers.get(&id(2)).ok_or("no such buffer")?;
                let encoder = self.encoders.get_mut(&id(1)).ok_or("no such encoder")?;
                encoder.clear_buffer(buffer, u64_of(a(3), 0), num(a(4)).map(|n| n as u64));
            }
            "copyTextureToBuffer" => {
                let (src, dst) = (a(2).ok_or("no source")?, a(3).ok_or("no destination")?);
                let texture = obj(src.get("texture")).and_then(|t| self.textures.get(&t)).ok_or("no such texture")?;
                let buffer = obj(dst.get("buffer")).and_then(|b| self.buffers.get(&b)).ok_or("no such buffer")?;
                let encoder = self.encoders.get_mut(&id(1)).ok_or("no such encoder")?;
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: u32_of(src.get("mipLevel"), 0),
                        origin: origin(src.get("origin")),
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: u64_of(dst.get("offset"), 0),
                            bytes_per_row: num(dst.get("bytesPerRow")).map(|n| n as u32),
                            rows_per_image: num(dst.get("rowsPerImage")).map(|n| n as u32),
                        },
                    },
                    extent(a(4)),
                );
            }
            "finish" => {
                let encoder = self.encoders.remove(&id(1)).ok_or("no such encoder")?;
                self.command_buffers.insert(id(2), encoder.finish());
            }
            "submit" => {
                let ids: Vec<u32> = a(1).and_then(Json::as_array).map(|l| l.iter().map(|v| u32_of(Some(v), 0)).collect()).unwrap_or_default();
                let buffers: Vec<wgpu::CommandBuffer> = ids.iter().filter_map(|i| self.command_buffers.remove(i)).collect();
                queue.submit(buffers);
            }
            "writeTexture" => {
                let dest = a(1).ok_or("no destination")?;
                let texture = obj(dest.get("texture")).and_then(|t| self.textures.get(&t)).ok_or("no such texture")?;
                let data = Self::bytes(payload, u64_of(a(2), 0), u64_of(a(3), 0))?;
                let layout = a(4);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: u32_of(dest.get("mipLevel"), 0),
                        origin: origin(dest.get("origin")),
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
                    wgpu::TexelCopyBufferLayout {
                        offset: u64_of(layout.and_then(|l| l.get("offset")), 0),
                        bytes_per_row: layout.and_then(|l| num(l.get("bytesPerRow"))).map(|n| n as u32),
                        rows_per_image: layout.and_then(|l| num(l.get("rowsPerImage"))).map(|n| n as u32),
                    },
                    extent(a(5)),
                );
            }
            "copyImage" => {
                let url = str_of(a(1)).unwrap_or("").to_string();
                let dest = a(2).ok_or("no destination")?;
                let texture = obj(dest.get("texture")).and_then(|t| self.textures.get(&t)).ok_or("no such texture")?;
                let (_, raw) = crate::fetch_bytes_for_page(&url)?;
                let mut rgba = image::load_from_memory(&raw).map_err(|e| e.to_string())?.to_rgba8();
                if a(3).and_then(Json::as_bool).unwrap_or(false) {
                    image::imageops::flip_vertical_in_place(&mut rgba);
                }
                let (w, h) = rgba.dimensions();
                let mut data = rgba.into_raw();
                if matches!(texture.format(), wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
                    for px in data.chunks_exact_mut(4) {
                        px.swap(0, 2);
                    }
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: origin(dest.get("origin")), aspect: wgpu::TextureAspect::All },
                    &data,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
            "configureCanvas" => {
                let format = str_of(a(3)).and_then(texture_format).unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
                self.canvases.insert(id(1), Canvas { flid: id(2), format, texture: None });
            }
            "canvasTexture" => {
                let canvas = self.canvases.get_mut(&id(1)).ok_or("canvas not configured")?;
                let (w, h) = (u32_of(a(3), 300).max(1), u32_of(a(4), 150).max(1));
                let stale = canvas.texture.as_ref().is_none_or(|(_, tw, th)| *tw != w || *th != h);
                if stale {
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("canvas"),
                        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: canvas.format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_SRC
                            | wgpu::TextureUsages::COPY_DST
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    });
                    canvas.texture = Some((texture, w, h));
                }
                let texture = canvas.texture.as_ref().unwrap().0.clone();
                self.textures.insert(id(2), texture);
            }
            "presentCanvas" => {
                let canvas = self.canvases.get(&id(1)).ok_or("canvas not configured")?;
                let Some((texture, w, h)) = canvas.texture.as_ref() else {
                    return Ok(String::new());
                };
                let (w, h) = (*w, *h);
                // Rows of a texture copy are padded to 256 bytes.
                let row = w * 4;
                let padded = row.div_ceil(256) * 256;
                let readback = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("canvas readback"),
                    size: (padded * h) as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
                    },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
                queue.submit([encoder.finish()]);
                let slice = readback.slice(..);
                slice.map_async(wgpu::MapMode::Read, |_| {});
                let _ = device.poll(wgpu::PollType::wait_indefinitely());
                let data = slice.get_mapped_range().map_err(|e| e.to_string())?;
                let bgra = matches!(canvas.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
                let mut rgba = Vec::with_capacity((row * h) as usize);
                for y in 0..h as usize {
                    for px in data[y * padded as usize..y * padded as usize + row as usize].chunks_exact(4) {
                        // The canvas is opaque, as `alphaMode: "opaque"` (the default) has it.
                        if bgra { rgba.extend_from_slice(&[px[2], px[1], px[0], 255]) } else { rgba.extend_from_slice(&[px[0], px[1], px[2], 255]) }
                    }
                }
                drop(data);
                readback.unmap();
                reply.frames.push(Frame { flid: canvas.flid, width: w, height: h, rgba });
            }
            _ => return Err("unsupported".into()),
        }
        Ok(String::new())
    }

    fn render_pipeline(&self, device: &wgpu::Device, desc: &Json) -> Result<wgpu::RenderPipeline, String> {
        let vertex = desc.get("vertex").ok_or("no vertex stage")?;
        let vertex_module = obj(vertex.get("module")).and_then(|m| self.modules.get(&m)).ok_or("no such module")?;
        // The vertex buffer layouts, then views of them.
        let mut layouts: Vec<(u64, wgpu::VertexStepMode, Vec<wgpu::VertexAttribute>)> = Vec::new();
        for b in vertex.get("buffers").and_then(Json::as_array).into_iter().flatten() {
            if b.is_null() {
                layouts.push((0, wgpu::VertexStepMode::Vertex, Vec::new()));
                continue;
            }
            let attributes = b
                .get("attributes")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .filter_map(|at| {
                    Some(wgpu::VertexAttribute {
                        format: vertex_format(str_of(at.get("format"))?)?,
                        offset: u64_of(at.get("offset"), 0),
                        shader_location: u32_of(at.get("shaderLocation"), 0),
                    })
                })
                .collect();
            let step = if str_of(b.get("stepMode")) == Some("instance") { wgpu::VertexStepMode::Instance } else { wgpu::VertexStepMode::Vertex };
            layouts.push((u64_of(b.get("arrayStride"), 0), step, attributes));
        }
        let buffers: Vec<Option<wgpu::VertexBufferLayout>> = layouts
            .iter()
            .map(|(stride, step, attributes)| Some(wgpu::VertexBufferLayout { array_stride: *stride, step_mode: *step, attributes }))
            .collect();
        let primitive = desc.get("primitive");
        let get = |k: &str| primitive.and_then(|p| str_of(p.get(k)));
        let topology = match get("topology") {
            Some("point-list") => wgpu::PrimitiveTopology::PointList,
            Some("line-list") => wgpu::PrimitiveTopology::LineList,
            Some("line-strip") => wgpu::PrimitiveTopology::LineStrip,
            Some("triangle-strip") => wgpu::PrimitiveTopology::TriangleStrip,
            _ => wgpu::PrimitiveTopology::TriangleList,
        };
        let strip_index_format = match get("stripIndexFormat") {
            Some("uint16") => Some(wgpu::IndexFormat::Uint16),
            Some("uint32") => Some(wgpu::IndexFormat::Uint32),
            _ => None,
        };
        let cull_mode = match get("cullMode") {
            Some("front") => Some(wgpu::Face::Front),
            Some("back") => Some(wgpu::Face::Back),
            _ => None,
        };
        let front_face = if get("frontFace") == Some("cw") { wgpu::FrontFace::Cw } else { wgpu::FrontFace::Ccw };
        let depth_stencil = match desc.get("depthStencil") {
            Some(ds) if !ds.is_null() => Some(wgpu::DepthStencilState {
                format: str_of(ds.get("format")).and_then(texture_format).ok_or("unsupported depth format")?,
                depth_write_enabled: ds.get("depthWriteEnabled").and_then(Json::as_bool),
                depth_compare: compare(str_of(ds.get("depthCompare"))),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            _ => None,
        };
        let fragment = desc.get("fragment").filter(|f| !f.is_null());
        let fragment_module = match fragment {
            Some(f) => Some(obj(f.get("module")).and_then(|m| self.modules.get(&m)).ok_or("no such module")?),
            None => None,
        };
        let targets: Vec<Option<wgpu::ColorTargetState>> = fragment
            .and_then(|f| f.get("targets"))
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .map(|t| {
                if t.is_null() {
                    return None;
                }
                Some(wgpu::ColorTargetState {
                    format: str_of(t.get("format")).and_then(texture_format)?,
                    blend: t.get("blend").filter(|b| !b.is_null()).map(|b| wgpu::BlendState {
                        color: blend_component(b.get("color")),
                        alpha: blend_component(b.get("alpha")),
                    }),
                    write_mask: wgpu::ColorWrites::from_bits_truncate(u32_of(t.get("writeMask"), 0xf)),
                })
            })
            .collect();
        let layout = obj(desc.get("layout")).and_then(|l| self.pipeline_layouts.get(&l));
        Ok(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout,
            vertex: wgpu::VertexState {
                module: vertex_module,
                entry_point: str_of(vertex.get("entryPoint")),
                compilation_options: Default::default(),
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState { topology, strip_index_format, front_face, cull_mode, ..Default::default() },
            depth_stencil,
            multisample: wgpu::MultisampleState {
                count: u32_of(desc.get("multisample").and_then(|m| m.get("count")), 1),
                ..Default::default()
            },
            fragment: fragment_module.map(|module| wgpu::FragmentState {
                module,
                entry_point: fragment.and_then(|f| str_of(f.get("entryPoint"))),
                compilation_options: Default::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        }))
    }

    /// Replay a pass the page recorded into its encoder.
    fn replay_pass(&mut self, pass: Pass) -> Result<(), String> {
        let Gpu { encoders, views, buffers, bind_groups, render_pipelines, compute_pipelines, .. } = self;
        let encoder = encoders.get_mut(&pass.encoder).ok_or("no such encoder")?;
        let arg = |c: &Json, i: usize| c.get(3 + i).cloned().unwrap_or(Json::Null);
        if pass.compute {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            for c in &pass.cmds {
                match str_of(c.get(2)).unwrap_or("") {
                    "setPipeline" => {
                        if let Some(pipeline) = obj(Some(&arg(c, 0))).and_then(|i| compute_pipelines.get(&i)) {
                            p.set_pipeline(pipeline);
                        }
                    }
                    "setBindGroup" => {
                        let offsets: Vec<u32> = arg(c, 2).as_array().map(|l| l.iter().map(|v| u32_of(Some(v), 0)).collect()).unwrap_or_default();
                        let group = obj(Some(&arg(c, 1))).and_then(|i| bind_groups.get(&i));
                        p.set_bind_group(u32_of(Some(&arg(c, 0)), 0), group, &offsets);
                    }
                    "dispatchWorkgroups" => {
                        p.dispatch_workgroups(u32_of(Some(&arg(c, 0)), 1), u32_of(Some(&arg(c, 1)), 1), u32_of(Some(&arg(c, 2)), 1));
                    }
                    _ => {}
                }
            }
            return Ok(());
        }
        let desc = &pass.desc;
        let mut colors = Vec::new();
        for att in desc.get("colorAttachments").and_then(Json::as_array).into_iter().flatten() {
            if att.is_null() {
                colors.push(None);
                continue;
            }
            let view = obj(att.get("view")).and_then(|v| views.get(&v)).ok_or("no such view")?;
            let load = if str_of(att.get("loadOp")) == Some("load") { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(color(att.get("clearValue"))) };
            let store = if str_of(att.get("storeOp")) == Some("discard") { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store };
            colors.push(Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: obj(att.get("resolveTarget")).and_then(|v| views.get(&v)),
                ops: wgpu::Operations { load, store },
            }));
        }
        let depth = match desc.get("depthStencilAttachment") {
            Some(d) if !d.is_null() => {
                let view = obj(d.get("view")).and_then(|v| views.get(&v)).ok_or("no such depth view")?;
                let ops = |load: Option<&str>, store: Option<&str>, clear: f32| wgpu::Operations {
                    load: if load == Some("load") { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(clear) },
                    store: if store == Some("discard") { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                };
                let depth_ops = str_of(d.get("depthLoadOp")).map(|load| {
                    ops(Some(load), str_of(d.get("depthStoreOp")), num(d.get("depthClearValue")).unwrap_or(1.0) as f32)
                });
                let stencil_ops = str_of(d.get("stencilLoadOp")).map(|load| wgpu::Operations {
                    load: if load == "load" { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(u32_of(d.get("stencilClearValue"), 0)) },
                    store: if str_of(d.get("stencilStoreOp")) == Some("discard") { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store },
                });
                Some(wgpu::RenderPassDepthStencilAttachment { view, depth_ops, stencil_ops })
            }
            _ => None,
        };
        let mut p = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &colors,
            depth_stencil_attachment: depth,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        for c in &pass.cmds {
            let n = |i: usize| num(Some(&arg(c, i)));
            match str_of(c.get(2)).unwrap_or("") {
                "setPipeline" => {
                    if let Some(pipeline) = obj(Some(&arg(c, 0))).and_then(|i| render_pipelines.get(&i)) {
                        p.set_pipeline(pipeline);
                    }
                }
                "setBindGroup" => {
                    let offsets: Vec<u32> = arg(c, 2).as_array().map(|l| l.iter().map(|v| u32_of(Some(v), 0)).collect()).unwrap_or_default();
                    let group = obj(Some(&arg(c, 1))).and_then(|i| bind_groups.get(&i));
                    p.set_bind_group(n(0).unwrap_or(0.0) as u32, group, &offsets);
                }
                "setVertexBuffer" => {
                    if let Some(buffer) = obj(Some(&arg(c, 1))).and_then(|i| buffers.get(&i)) {
                        let offset = n(2).unwrap_or(0.0) as u64;
                        let slice = match n(3) {
                            Some(size) => buffer.slice(offset..offset + size as u64),
                            None => buffer.slice(offset..),
                        };
                        p.set_vertex_buffer(n(0).unwrap_or(0.0) as u32, slice);
                    }
                }
                "setIndexBuffer" => {
                    if let Some(buffer) = obj(Some(&arg(c, 0))).and_then(|i| buffers.get(&i)) {
                        let format = if arg(c, 1).as_str() == Some("uint32") { wgpu::IndexFormat::Uint32 } else { wgpu::IndexFormat::Uint16 };
                        let offset = n(2).unwrap_or(0.0) as u64;
                        let slice = match n(3) {
                            Some(size) => buffer.slice(offset..offset + size as u64),
                            None => buffer.slice(offset..),
                        };
                        p.set_index_buffer(slice, format);
                    }
                }
                "setViewport" => p.set_viewport(
                    n(0).unwrap_or(0.0) as f32,
                    n(1).unwrap_or(0.0) as f32,
                    n(2).unwrap_or(0.0) as f32,
                    n(3).unwrap_or(0.0) as f32,
                    n(4).unwrap_or(0.0) as f32,
                    n(5).unwrap_or(1.0) as f32,
                ),
                "setScissorRect" => p.set_scissor_rect(
                    n(0).unwrap_or(0.0) as u32,
                    n(1).unwrap_or(0.0) as u32,
                    n(2).unwrap_or(0.0) as u32,
                    n(3).unwrap_or(0.0) as u32,
                ),
                "setBlendConstant" => p.set_blend_constant(color(Some(&arg(c, 0)))),
                "setStencilReference" => p.set_stencil_reference(n(0).unwrap_or(0.0) as u32),
                "draw" => {
                    let count = n(0).unwrap_or(0.0) as u32;
                    let instances = n(1).unwrap_or(1.0) as u32;
                    let first = n(2).unwrap_or(0.0) as u32;
                    let first_instance = n(3).unwrap_or(0.0) as u32;
                    p.draw(first..first + count, first_instance..first_instance + instances);
                }
                "drawIndexed" => {
                    let count = n(0).unwrap_or(0.0) as u32;
                    let instances = n(1).unwrap_or(1.0) as u32;
                    let first = n(2).unwrap_or(0.0) as u32;
                    let base = n(3).unwrap_or(0.0) as i32;
                    let first_instance = n(4).unwrap_or(0.0) as u32;
                    p.draw_indexed(first..first + count, base, first_instance..first_instance + instances);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn bind_group_layout_entry(e: &Json) -> Option<wgpu::BindGroupLayoutEntry> {
    let binding = u32_of(e.get("binding"), 0);
    let visibility = wgpu::ShaderStages::from_bits_truncate(u32_of(e.get("visibility"), 0));
    let ty = if let Some(b) = e.get("buffer").filter(|b| !b.is_null()) {
        let ty = match str_of(b.get("type")) {
            Some("storage") => wgpu::BufferBindingType::Storage { read_only: false },
            Some("read-only-storage") => wgpu::BufferBindingType::Storage { read_only: true },
            _ => wgpu::BufferBindingType::Uniform,
        };
        wgpu::BindingType::Buffer {
            ty,
            has_dynamic_offset: b.get("hasDynamicOffset").and_then(Json::as_bool).unwrap_or(false),
            min_binding_size: num(b.get("minBindingSize")).and_then(|n| NonZeroU64::new(n as u64)),
        }
    } else if let Some(s) = e.get("sampler").filter(|s| !s.is_null()) {
        wgpu::BindingType::Sampler(match str_of(s.get("type")) {
            Some("non-filtering") => wgpu::SamplerBindingType::NonFiltering,
            Some("comparison") => wgpu::SamplerBindingType::Comparison,
            _ => wgpu::SamplerBindingType::Filtering,
        })
    } else if let Some(t) = e.get("texture").filter(|t| !t.is_null()) {
        wgpu::BindingType::Texture {
            sample_type: match str_of(t.get("sampleType")) {
                Some("unfilterable-float") => wgpu::TextureSampleType::Float { filterable: false },
                Some("depth") => wgpu::TextureSampleType::Depth,
                Some("sint") => wgpu::TextureSampleType::Sint,
                Some("uint") => wgpu::TextureSampleType::Uint,
                _ => wgpu::TextureSampleType::Float { filterable: true },
            },
            view_dimension: view_dimension(str_of(t.get("viewDimension"))).unwrap_or(wgpu::TextureViewDimension::D2),
            multisampled: t.get("multisampled").and_then(Json::as_bool).unwrap_or(false),
        }
    } else if let Some(t) = e.get("storageTexture").filter(|t| !t.is_null()) {
        wgpu::BindingType::StorageTexture {
            access: match str_of(t.get("access")) {
                Some("read-only") => wgpu::StorageTextureAccess::ReadOnly,
                Some("read-write") => wgpu::StorageTextureAccess::ReadWrite,
                _ => wgpu::StorageTextureAccess::WriteOnly,
            },
            format: str_of(t.get("format")).and_then(texture_format)?,
            view_dimension: view_dimension(str_of(t.get("viewDimension"))).unwrap_or(wgpu::TextureViewDimension::D2),
        }
    } else {
        return None;
    };
    Some(wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None })
}
