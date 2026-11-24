pub mod backend;
pub mod bundle;
mod canvas;
mod canvas2d;
pub mod engine;
pub mod esm;
mod html_seed;
pub mod http_cache;
pub mod sql;

#[cfg(target_os = "android")]
pub mod jni_bridge;
#[cfg(target_os = "android")]
mod webgl;

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use crate::engine::{HostDeviceInfo, SoftnMobileEngine};

    fn make_zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, content) in files {
                zw.start_file(*name, opts).unwrap();
                zw.write_all(content.as_bytes()).unwrap();
            }
            zw.finish().unwrap();
        }
        buf
    }

    fn temp_dir(label: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "softn_mobile_test_{label}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn fullstack_php_sqlite_and_device_bridge_renders_to_rgba() {
        let bundle = make_zip(&[
            (
                "manifest.json",
                r#"{
                  "id": "test.fieldkit",
                  "name": "Fieldkit Test",
                  "version": "1.0.0",
                  "engine": 2,
                  "main": "index.html",
                  "backend": {
                    "runtime": "php",
                    "entry": "backend/index.php",
                    "mode": "api",
                    "routes": ["/api/"]
                  },
                  "databases": [
                    { "name": "app", "file": "app.db", "seedSql": "backend/schema.sql" }
                  ]
                }"#,
            ),
            (
                "permission.json",
                r#"{
                  "permissions": {
                    "haptics": { "enabled": true },
                    "bluetooth": { "enabled": true, "services": ["180d"] }
                  }
                }"#,
            ),
            (
                "backend/schema.sql",
                "CREATE TABLE IF NOT EXISTS logs (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL);
                 INSERT INTO logs (title) VALUES ('Seeded Sensor Check');",
            ),
            (
                "backend/index.php",
                r#"<?php
header('Content-Type: application/json');
$pdo = new PDO('sqlite:app.db');
$uri = $_SERVER['REQUEST_URI'] ?? '/';
$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
if ($method === 'POST' && str_starts_with($uri, '/api/logs')) {
    $data = json_decode(file_get_contents('php://input'), true);
    $stmt = $pdo->prepare('INSERT INTO logs (title) VALUES (?)');
    $stmt->execute([$data['title'] ?? 'Untitled']);
}
$rows = $pdo->query('SELECT id, title FROM logs ORDER BY id ASC')->fetchAll(PDO::FETCH_ASSOC);
echo json_encode(['ok' => true, 'logs' => $rows]);
"#,
            ),
            (
                "index.html",
                r#"<!DOCTYPE html>
<html>
<head>
  <link rel="stylesheet" href="softn:ui.css">
  <script type="module" src="./app.js"></script>
</head>
<body>
  <div id="root">Loading...</div>
</body>
</html>"#,
            ),
            (
                "app.js",
                r#"
import { getInfo } from 'softn:device';
import { impact } from 'softn:haptics';
import { scan } from 'softn:bluetooth';

async function init() {
  const info = await getInfo();
  const res = await fetch('/api/logs');
  const data = await res.json();
  const root = document.getElementById('root');
  root.innerHTML = '<h1 id="title">' + info.platform + ' / ' + info.engine + '</h1>' +
    '<p id="count">Logs: ' + data.logs.length + ' (' + data.logs[0].title + ')</p>' +
    '<button id="btn">Pulse</button>';
  // Trigger allowed haptic call (queues pending native bridge call)
  impact('heavy');
  // Disallowed BLE service should be rejected natively by Rust
  scan({ services: ['180f'] }).catch((err) => {
    const p = document.createElement('p');
    p.id = 'ble-err';
    p.textContent = err.message;
    root.appendChild(p);
  });
}
init();
"#,
            ),
        ]);

        let dir = temp_dir("php_sqlite");
        let mut eng = SoftnMobileEngine::load_bundle(
            &bundle,
            &dir,
            400,
            600,
            2.0,
            HostDeviceInfo::default(),
        )
        .expect("load_bundle failed");

        let html = eng.current_rendered_html();
        assert!(html.contains("android / zipp-browser-android"), "HTML was: {html}");
        assert!(html.contains("Logs: 1 (Seeded Sensor Check)"), "HTML was: {html}");
        assert!(html.contains("allowlist"), "Expected BLE allowlist rejection in HTML: {html}");

        // Haptics call should be queued for Android Kotlin bridge
        let pending = eng.drain_pending_calls();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].namespace, "haptics");
        assert_eq!(pending[0].method, "impact");

        // Render via zipp-dom + zipp-renderer TinySkia into RGBA8888 buffer
        let mut rgba = vec![0u8; 400 * 600 * 4];
        assert!(eng.render_rgba(&mut rgba));
        assert!(rgba.iter().any(|&b| b != 0), "Rendered framebuffer should not be blank");
    }

    #[test]
    fn loads_real_opal_sales_fullstack_bundle_if_present() {
        let opal_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/opal-business/bundles/opal-sales.softn");
        if !opal_path.exists() {
            return;
        }
        let bytes = std::fs::read(&opal_path).unwrap();
        let dir = temp_dir("opal_sales");
        let mut eng = SoftnMobileEngine::load_bundle(
            &bytes,
            &dir,
            412,
            915,
            2.0,
            HostDeviceInfo::default(),
        )
        .expect("opal-sales.softn failed to boot");

        let html = eng.current_rendered_html();
        assert!(
            html.contains("Sales") || html.contains("Invoice") || html.contains("Opal"),
            "Unexpected opal-sales rendered HTML: {}",
            &html[..html.len().min(600)]
        );
        let mut rgba = vec![0u8; 412 * 915 * 4];
        assert!(eng.render_rgba(&mut rgba));
    }

    #[test]
    fn loads_mobile_fieldkit_fullstack_bundle_and_renders_inspections() {
        let fk_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/mobile-fieldkit/mobile-fieldkit.softn");
        if !fk_path.exists() {
            return;
        }
        let bytes = std::fs::read(&fk_path).unwrap();
        let dir = temp_dir("mobile_fieldkit");
        let host_info = HostDeviceInfo {
            os_version: "Android 16 (API 36)".into(),
            model: "Google sdk_gphone64_x86_64".into(),
            ..Default::default()
        };
        let mut eng = SoftnMobileEngine::load_bundle(
            &bytes,
            &dir,
            1080,
            2219,
            2.625,
            host_info,
        )
        .expect("mobile-fieldkit.softn failed to boot");

        let html = eng.current_rendered_html();
        assert!(html.contains("FieldKit"), "HTML missing title: {html}");
        assert!(
            html.contains("Turbine Bearing Temperature OK"),
            "HTML missing SQLite PHP rows: {html}"
        );
        assert!(html.contains("2 / 3 verified"), "HTML missing stats: {html}");
        let mut rgba = vec![0u8; 1080 * 2219 * 4];
        assert!(eng.render_rgba(&mut rgba));
        // Ensure multiple colors were painted (not just uniform dark background)
        let first_px = &rgba[0..4];
        let diff_pixels = rgba.chunks_exact(4).filter(|px| *px != first_px).count();
        assert!(
            diff_pixels > 5000,
            "Expected rich painted UI pixels, found only {diff_pixels} distinct pixels"
        );

        // Tap "+ Quick Log" button (inside viewport; note status bar isn't in engine coords, so y ~ 720px on 1080x2219)
        let tap_res = eng.on_tap(855.0, 720.0);
        let html_after = eng.current_rendered_html();
        assert!(
            html_after.contains("Field Check #1")
                && html_after.contains("Saved #4 to persistent SQLite via PHP 8")
                && html_after.contains("2 / 4 verified"),
            "Quick Log tap did not persist new inspection with lastInsertId! dirty={}, html={}",
            tap_res.dirty,
            html_after
        );
    }

    #[test]
    fn loads_real_mining_system_rust_sqlite_fullstack_bundle_if_present() {
        let pit_path = PathBuf::from(r"E:\repos\mining_projects\mining_system\dist\pit-management.softn");
        if !pit_path.exists() {
            return;
        }
        let bytes = std::fs::read(&pit_path).unwrap();
        let dir = temp_dir("pit_management");
        let mut eng = SoftnMobileEngine::load_bundle(
            &bytes,
            &dir,
            1080,
            2219,
            2.625,
            HostDeviceInfo::default(),
        )
        .expect("pit-management.softn failed to boot");

        // 1. Verify Rust Axum + SQLite 3 backend APIs work natively in-process
        let be = eng.backend.as_ref().expect("Expected Rust backend");
        let health = be.handle_request(crate::backend::BackendHttpRequest {
            method: "GET".into(),
            path: "/api/v1/health".into(),
            query: String::new(),
            headers: vec![],
            body: vec![],
        });
        assert_eq!(
            health.status,
            200,
            "Health check failed: {}",
            String::from_utf8_lossy(&health.body)
        );

        let setup = be.handle_request(crate::backend::BackendHttpRequest {
            method: "GET".into(),
            path: "/api/v1/setup/status".into(),
            query: String::new(),
            headers: vec![],
            body: vec![],
        });
        let setup_json: serde_json::Value = serde_json::from_slice(&setup.body).unwrap();
        assert!(
            setup_json["sites_count"].as_u64().unwrap_or(0) >= 2,
            "Expected seeded sites in SQLite: {setup_json}"
        );

        let login = be.handle_request(crate::backend::BackendHttpRequest {
            method: "POST".into(),
            path: "/api/v1/auth/login".into(),
            query: String::new(),
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: br#"{"email":"admin@pit.management","password":"ChangeMe!2026"}"#.to_vec(),
        });
        assert_eq!(
            login.status,
            200,
            "Login failed: {}",
            String::from_utf8_lossy(&login.body)
        );
        let login_json: serde_json::Value = serde_json::from_slice(&login.body).unwrap();
        let token = login_json["access_token"].as_str().unwrap().to_string();

        let dash = be.handle_request(crate::backend::BackendHttpRequest {
            method: "GET".into(),
            path: "/api/v1/dashboard".into(),
            query: "site_id=1".into(),
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            body: vec![],
        });
        assert_eq!(
            dash.status,
            200,
            "Dashboard failed: {}",
            String::from_utf8_lossy(&dash.body)
        );
        let dash_json: serde_json::Value = serde_json::from_slice(&dash.body).unwrap();
        assert!(
            dash_json["kpis"]["loads_24h"].as_u64().unwrap_or(0) > 0,
            "Expected seeded 24h loads: {dash_json}"
        );

        // 2. Verify React frontend rendered Control Room Sign-In and paints cleanly to RGBA
        let html = eng.current_rendered_html();
        let logs = eng.logs.lock().unwrap().clone();
        assert!(
            html.contains("Control Room Sign-In") || html.contains("pit.software"),
            "Unexpected pit-management rendered HTML (logs={logs:?}): {}",
            &html[..html.len().min(1200)]
        );
        let mut rgba = vec![0u8; 1080 * 2219 * 4];
        assert!(eng.render_rgba(&mut rgba));
        let first_px = &rgba[0..4];
        let diff_pixels = rgba.chunks_exact(4).filter(|px| *px != first_px).count();
        assert!(
            diff_pixels > 5000,
            "Expected rich painted UI for pit.software, found only {diff_pixels} distinct pixels"
        );

        // 2b. Tap the "Sign In to pit.software" submit button via real hit-test and verify
        //     authenticated Cockpit dashboard loads live data from Rust Axum + SQLite 3!
        let submit_pos = html
            .find("Sign In to pit.software")
            .and_then(|idx| {
                // Find the `<button ...>` tag enclosing "Sign In to pit.software"
                let prefix = &html[..idx];
                let btn_start = prefix.rfind("<button")?;
                let btn_slice = &prefix[btn_start..];
                let flid_key = "data-flid=\"";
                let start = btn_slice.find(flid_key)? + flid_key.len();
                let end = btn_slice[start..].find('"')?;
                let flid: u32 = btn_slice[start..start + end].parse().ok()?;
                eprintln!("[pit-test] Found <button> flid={flid}: {}", &btn_slice[..btn_slice.len().min(180)]);
                eng.find_flid_physical_center(flid)
            });
        let (tx, ty) = submit_pos.expect("Could not locate Sign In button coordinates");
        eprintln!("[pit-test] Tapping Sign In button at ({tx:.1}, {ty:.1})");
        let tap_res = eng.on_tap(tx, ty);
        eprintln!("[pit-test] tap_res={tap_res:?}");
        for _ in 0..5 {
            let _ = eng.tick();
        }
        let cockpit_html = eng.current_rendered_html();
        let logs_after = eng.logs.lock().unwrap().clone();
        eprintln!("[pit-test] Logs after login: {logs_after:?}");
        assert!(
            !cockpit_html.contains("Control Room Sign-In"),
            "Still on login screen after tapping Sign In! form_snippet={}, logs={logs_after:?}",
            cockpit_html.find("<form").map(|i| &cockpit_html[i..(i+800).min(cockpit_html.len())]).unwrap_or(&cockpit_html[..cockpit_html.len().min(800)])
        );
        assert!(
            !cockpit_html.contains("Module Recovered from Runtime Error"),
            "Cockpit plugin hit ErrorBoundary! logs={logs_after:?}"
        );
        assert!(
            cockpit_html.contains("24h Production")
                && cockpit_html.contains("Fleet OEE")
                && cockpit_html.contains("Completed Cycles"),
            "Expected live SQLite Cockpit KPIs (not 'Loading shift cockpit...'), got: {}",
            &cockpit_html[..cockpit_html.len().min(2000)]
        );
        assert!(eng.render_rgba(&mut rgba));

        // 2c. Verify Live 3D Mine Map <canvas> actually rendered rich 3D pixels via Canvas 2D!
        let canvases = eng.canvas_frame_summary();
        eprintln!("[pit-test] Cockpit canvases (w, h, varied_px): {canvases:?}");
        assert!(
            canvases.iter().any(|&(w, h, varied)| w >= 600 && h >= 500 && varied > 5000),
            "Expected Live 3D Mine Map <canvas> to render rich 3D scene, got: {canvases:?}"
        );
        if std::env::var("SOFTN_BENCH").is_ok() {
            // 1. Scroll down so the 3D Mine Map <canvas> is FULLY VISIBLE on screen
            eng.on_scroll(2500.0);
            for _ in 0..5 {
                let _ = eng.tick();
                let _ = eng.render_rgba(&mut rgba);
            }
            let mut tick_us = 0u128;
            let mut paint_us = 0u128;
            for _ in 0..10 {
                let t0 = std::time::Instant::now();
                let _ = eng.tick();
                let t1 = std::time::Instant::now();
                let _ = eng.render_rgba(&mut rgba);
                let t2 = std::time::Instant::now();
                tick_us += (t1 - t0).as_micros();
                paint_us += (t2 - t1).as_micros();
            }
            let avg_tick = tick_us as f64 / 10_000.0;
            let avg_paint = paint_us as f64 / 10_000.0;
            let avg_total = avg_tick + avg_paint;
            eprintln!(
                "[pit-bench] visible 3D map (10-frame avg): tick={avg_tick:.2}ms  paint={avg_paint:.2}ms  total={avg_total:.2}ms ({:.1} FPS)",
                1000.0 / avg_total.max(0.01)
            );
            // 2. Also measure full-page repaint speed (e.g. during scroll or DOM HUD updates)
            let mut full_us = 0u128;
            for i in 0..5 {
                eng.on_scroll(if i % 2 == 0 { 10.0 } else { -10.0 });
                let t0 = std::time::Instant::now();
                let _ = eng.render_rgba(&mut rgba);
                full_us += t0.elapsed().as_micros();
            }
            eprintln!(
                "[pit-bench] full-page repaint (5-frame avg): paint={:.2}ms",
                full_us as f64 / 5_000.0
            );
            eng.on_scroll(-2500.0);
        }

        let save_png = |rgba_buf: &[u8], name: &str| {
            if let Ok(dir) = std::env::var("SOFTN_DUMP_SCREENSHOTS") {
                let path = std::path::Path::new(&dir).join(name);
                // TinySkia stores premultiplied RGBA; un-premultiply for crisp PNG output
                let mut straight = rgba_buf.to_vec();
                for px in straight.chunks_exact_mut(4) {
                    let a = px[3] as u32;
                    if a > 0 && a < 255 {
                        px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
                        px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
                        px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
                    }
                }
                if let Some(img) = image::RgbaImage::from_raw(1080, 2219, straight) {
                    let _ = img.save(&path);
                }
            }
        };
        if std::env::var("SOFTN_DUMP_SCREENSHOTS").is_ok() {
            assert!(eng.render_rgba(&mut rgba));
            save_png(&rgba, "v2_cockpit_top.png");
            eng.on_scroll(2500.0);
            assert!(eng.render_rgba(&mut rgba));
            save_png(&rgba, "v2_cockpit_3dmap.png");
            eng.on_scroll(-2500.0);
        }

        // 2d. Navigate to Map tab and verify Live 3D Mine Map full view
        if std::env::var("SOFTN_DUMP_SCREENSHOTS").is_ok() {
            let map_flid = cockpit_html.rfind(">Map<").and_then(|idx| {
                let prefix = &cockpit_html[..idx];
                let btn_start = prefix.rfind("<button")?;
                let slice = &prefix[btn_start..];
                let k = "data-flid=\"";
                let s = slice.find(k)? + k.len();
                let e = slice[s..].find('"')?;
                slice[s..s + e].parse::<u32>().ok()
            });
            if let Some(mflid) = map_flid {
                if let Some((mx, my)) = eng.find_flid_physical_center(mflid) {
                    let _ = eng.on_tap(mx, my);
                    for _ in 0..3 {
                        let _ = eng.tick();
                    }
                    assert!(eng.render_rgba(&mut rgba));
                    save_png(&rgba, "v2_map_tab.png");
                }
            }
        }

        // 2e. Navigate to Dispatch and verify routes + select dropdowns render
        let nav_html = eng.current_rendered_html();
        let dispatch_flid = nav_html
            .rfind(">Dispatch<")
            .and_then(|idx| {
                let prefix = &nav_html[..idx];
                let btn_start = prefix.rfind("<button")?;
                let slice = &prefix[btn_start..];
                let k = "data-flid=\"";
                let s = slice.find(k)? + k.len();
                let e = slice[s..].find('"')?;
                slice[s..s + e].parse::<u32>().ok()
            });
        if let Some(dflid) = dispatch_flid {
            if let Some((dx, dy)) = eng.find_flid_physical_center(dflid) {
                let _ = eng.on_tap(dx, dy);
                for _ in 0..3 {
                    let _ = eng.tick();
                }
                let disp_html = eng.current_rendered_html();
                assert!(
                    disp_html.contains("CRUSH-01") && disp_html.contains("HT-201"),
                    "Expected Dispatch haul routes and trucks"
                );
                assert!(eng.render_rgba(&mut rgba));
                save_png(&rgba, "v2_dispatch_top.png");
                if std::env::var("SOFTN_DUMP_SCREENSHOTS").is_ok() {
                    eng.on_scroll(6500.0);
                    assert!(eng.render_rgba(&mut rgba));
                    save_png(&rgba, "v2_dispatch_dock.png");
                    eng.on_scroll(-6500.0);
                }
            }
        }

        // 3. Verify SQLite + `.pit.secret` persist across cold restart (same JWT still valid!)
        drop(eng);
        let eng2 = SoftnMobileEngine::load_bundle(
            &bytes,
            &dir,
            1080,
            2219,
            2.625,
            HostDeviceInfo::default(),
        )
        .expect("pit-management.softn cold restart failed");
        let be2 = eng2.backend.as_ref().unwrap();
        let dash_after_restart = be2.handle_request(crate::backend::BackendHttpRequest {
            method: "GET".into(),
            path: "/api/v1/dashboard".into(),
            query: "site_id=1".into(),
            headers: vec![("Authorization".into(), format!("Bearer {token}"))],
            body: vec![],
        });
        assert_eq!(
            dash_after_restart.status,
            200,
            "JWT failed after cold restart (`.pit.secret` persistence?): {}",
            String::from_utf8_lossy(&dash_after_restart.body)
        );
    }

    #[test]
    fn browser_parity_binary_fetch_cookies_sessions_shadow_dom_and_permissions() {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            let add = |zw: &mut zip::ZipWriter<std::io::Cursor<&mut Vec<u8>>>, name: &str, data: &[u8]| {
                zw.start_file(name, opts).unwrap();
                zw.write_all(data).unwrap();
            };
            add(
                &mut zw,
                "manifest.json",
                br#"{
                  "id": "test.parity",
                  "name": "Mobile Parity Test",
                  "version": "1.0.0",
                  "engine": 2,
                  "main": "index.html",
                  "backend": {
                    "runtime": "php",
                    "entry": "backend/index.php",
                    "mode": "api",
                    "routes": ["/api/"]
                  }
                }"#,
            );
            add(
                &mut zw,
                "permission.json",
                br#"{
                  "permissions": {
                    "location": { "enabled": true },
                    "bluetooth": { "enabled": true },
                    "accel": { "enabled": true }
                  }
                }"#,
            );
            // Binary GLB header + high bytes (0x00, 0xFF, 0x80) that UTF-8 lossy decoding would corrupt
            add(&mut zw, "assets/model.glb", &[0x67, 0x6c, 0x54, 0x46, 0x00, 0xff, 0x80, 0x42]);
            add(
                &mut zw,
                "backend/index.php",
                br#"<?php
header('Content-Type: application/json');
$uri = $_SERVER['REQUEST_URI'] ?? '/';
$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';
$cookie = $_SERVER['HTTP_COOKIE'] ?? '';
if ($method === 'POST' && $uri === '/api/login') {
    header('Set-Cookie: sid=mobile_sess_99; Path=/; HttpOnly');
    header('Set-Cookie: role=inspector; Path=/; Max-Age=3600', false);
    echo json_encode(['ok' => true]);
    exit;
}
if ($uri === '/api/me') {
    echo json_encode(['cookie' => $cookie]);
    exit;
}
http_response_code(404);
echo json_encode(['error' => 'not found']);
"#,
            );
            add(
                &mut zw,
                "index.html",
                br#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>Parity</title></head>
<body>
  <div id="app"></div>
  <script>
    (async function () {
      var out = {};
      // 1. Binary GLB fetch + Blob + URL.createObjectURL round-trip (zero UTF-8 corruption)
      var glbResp = await fetch('/assets/model.glb');
      var glbBytes = new Uint8Array(await glbResp.arrayBuffer());
      var blob = new Blob([glbBytes], { type: 'model/gltf-binary' });
      var blobUrl = URL.createObjectURL(blob);
      var blobFetched = new Uint8Array(await (await fetch(blobUrl)).arrayBuffer());
      URL.revokeObjectURL(blobUrl);
      out.binaryOk = glbBytes.length === 8 && glbBytes[4] === 0 && glbBytes[5] === 255 && glbBytes[6] === 128
        && blobFetched.length === 8 && blobFetched[5] === 255 && blobFetched[7] === 0x42;

      // 2. Shadow DOM + adoptedStyleSheets + DOMParser + !important
      var host = document.createElement('div');
      host.id = 'shadow-host';
      var sr = host.attachShadow({ mode: 'open' });
      var sheet = new CSSStyleSheet();
      sheet.replaceSync(':host { display: block; } .badge { color: #22c55e !important; }');
      sr.adoptedStyleSheets = [sheet];
      var parsed = new DOMParser().parseFromString('<span class="badge">SHADOW_OK</span>', 'text/html');
      sr.appendChild(parsed.body.firstChild);
      document.getElementById('app').appendChild(host);

      // 3. Cookies & sessions against embedded PHP backend
      await fetch('/api/login', { method: 'POST' });
      var me = await (await fetch('/api/me')).json();
      var omit = await (await fetch('/api/me', { credentials: 'omit' })).json();
      out.cookieSent = String(me.cookie || '').indexOf('sid=mobile_sess_99') >= 0
        && String(me.cookie || '').indexOf('role=inspector') >= 0;
      out.omitClean = String(omit.cookie || '') === '';
      out.httpOnlyHidden = document.cookie.indexOf('sid=') < 0 && document.cookie.indexOf('role=inspector') >= 0;

      // 4. sessionStorage + MutationObserver
      sessionStorage.setItem('phase', 'mobile-ok');
      out.sessionOk = sessionStorage.getItem('phase') === 'mobile-ok';
      var moHit = false;
      var mo = new MutationObserver(function () { moHit = true; });
      mo.observe(host, { attributes: true });
      host.setAttribute('data-ready', '1');
      await Promise.resolve();
      mo.disconnect();
      out.moOk = moHit;

      // 5. Permission gating (camera NOT declared -> denied; location declared -> queued)
      var camPerm = await navigator.permissions.query({ name: 'camera' });
      out.camDenied = camPerm.state === 'denied';
      var camErr = '';
      try { await softn.camera.takePhoto(); } catch (e) { camErr = String(e.message || e); }
      out.camBlocked = camErr.indexOf('not permitted') >= 0;

      // Queue a permitted GPS + BLE + sensor call to verify native dispatch
      navigator.geolocation.getCurrentPosition(function () {}, function () {}, { enableHighAccuracy: true });
      softn.sensors.getMotion().catch(function () {});

      var pre = document.createElement('pre');
      pre.id = 'results';
      pre.textContent = JSON.stringify(out);
      document.getElementById('app').appendChild(pre);
    })();
  </script>
</body>
</html>"#,
            );
            zw.finish().unwrap();
        }

        let dir = temp_dir("parity");
        let mut eng = SoftnMobileEngine::load_bundle(
            &buf,
            &dir,
            412,
            915,
            1.0,
            HostDeviceInfo::default(),
        )
        .expect("load parity bundle");
        let mut pending = eng.drain_pending_calls();
        let tick_res = eng.tick();
        pending.extend(tick_res.pending_calls);

        let html = eng.current_rendered_html();
        assert!(html.contains("SHADOW_OK"), "Shadow DOM content missing from rendered HTML: {html}");
        assert!(html.contains("data-zsr="), "Shadow root scope attribute missing: {html}");
        assert!(html.contains("\"binaryOk\":true"), "Binary GLB/Blob check failed: {html}");
        assert!(html.contains("\"cookieSent\":true"), "Session cookie not sent to PHP backend: {html}");
        assert!(html.contains("\"omitClean\":true"), "credentials:omit leaked cookie: {html}");
        assert!(html.contains("\"httpOnlyHidden\":true"), "HttpOnly cookie check failed: {html}");
        assert!(html.contains("\"sessionOk\":true"), "sessionStorage failed: {html}");
        assert!(html.contains("\"moOk\":true"), "MutationObserver failed: {html}");
        assert!(html.contains("\"camDenied\":true"), "Undeclared camera permission query should be denied: {html}");
        assert!(html.contains("\"camBlocked\":true"), "Undeclared camera call must be blocked in Rust: {html}");

        // Verify permitted location + sensor calls were queued for Android DeviceBridge
        pending.extend(eng.drain_pending_calls());
        assert!(
            pending.iter().any(|c| c.namespace == "location" && c.method == "getCurrentPosition"),
            "Expected location call in pending queue: {pending:?}"
        );
        assert!(
            pending.iter().any(|c| c.namespace == "sensors" && c.method == "getMotion"),
            "Expected sensors.getMotion call in pending queue: {pending:?}"
        );

        // Verify clear_cache and clear_site_data work cleanly
        let cc: serde_json::Value = serde_json::from_str(&eng.clear_cache()).unwrap();
        assert_eq!(cc["ok"], true);
        let csd: serde_json::Value = serde_json::from_str(&eng.clear_site_data()).unwrap();
        assert_eq!(csd["ok"], true);
    }

    #[test]
    fn supports_2d_scroll_idle_timer_reporting_and_batched_storage() {
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts = zip::write::SimpleFileOptions::default();
            zw.start_file("manifest.json", opts).unwrap();
            zw.write_all(
                br#"{
                "name": "Scroll & Idle Test",
                "version": "1.0.0",
                "main": "index.html"
            }"#,
            )
            .unwrap();
            zw.start_file("index.html", opts).unwrap();
            zw.write_all(
                br#"<!doctype html>
<html>
<head>
  <style>
    body { margin: 0; padding: 0; }
    #wide { width: 1200px; height: 2400px; background: #1e293b; }
    #btn { width: 180px; height: 48px; }
  </style>
</head>
<body>
  <div id="wide">
    <button id="btn">Count: 0</button>
    <input id="note" type="text" placeholder="Note..." />
  </div>
  <script>
    var count = 0;
    var btn = document.getElementById("btn");
    btn.addEventListener("click", function () {
      count++;
      btn.textContent = "Count: " + count;
      localStorage.setItem("k1", "v" + count);
      localStorage.setItem("k2", "saved-" + count);
    });
  </script>
</body>
</html>"#,
            )
            .unwrap();
            zw.finish().unwrap();
        }

        let dir = temp_dir("scroll_idle");
        let mut eng = SoftnMobileEngine::load_bundle(
            &buf,
            &dir,
            400,
            800,
            1.0,
            HostDeviceInfo::default(),
        )
        .expect("load scroll_idle bundle");

        // No canvas and no timers scheduled -> idle next_timer_ms == -1, has_canvas == false
        assert!(!eng.has_canvas(), "Static DOM should report has_canvas=false");
        let idle_tick = eng.tick();
        assert!(!idle_tick.has_canvas);
        assert!(!idle_tick.wants_raf);
        assert_eq!(idle_tick.next_timer_ms, -1, "Idle app should report next_timer_ms=-1");

        // Rapid taps should update state and persist localStorage via persistent KV connection
        let tap1 = eng.on_tap(60.0, 24.0);
        assert!(tap1.dirty);
        assert!(!tap1.has_canvas);
        assert!(eng.current_rendered_html().contains("Count: 1"));
        assert_eq!(eng.storage.load_local_storage().get("k2").map(String::as_str), Some("saved-1"));

        // Tapping and typing into an uncontrolled <input> must mark DOM mutated & re-render value
        let tap_input = eng.on_tap(250.0, 24.0);
        assert!(tap_input.wants_ime, "Expected input tap to open IME");
        let typed = eng.on_text_input("Turbine OK");
        assert!(typed.dirty, "Typing into uncontrolled input must re-render");
        assert!(
            eng.current_rendered_html().contains("Turbine OK"),
            "Rendered HTML missing typed input value: {}",
            eng.current_rendered_html()
        );

        // 2D scrolling works on both Y and X axes and updates viewport cleanly
        assert!(eng.on_scroll_2d(0.0, 120.0), "Expected vertical scroll to move");
        assert!(eng.on_scroll_2d(150.0, 0.0), "Expected horizontal scroll to move");
        assert!(eng.on_scroll_2d(-150.0, -120.0), "Expected scrolling back to origin to move");

        // Measure 15 taps + 50 scroll steps to verify responsive layout/scroll pipeline
        let t0 = std::time::Instant::now();
        for _ in 0..15 {
            let _ = eng.on_tap(60.0, 24.0);
        }
        let tap_ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert!(eng.current_rendered_html().contains("Count: 16"));

        let s0 = std::time::Instant::now();
        for i in 0..50 {
            let dy = if i < 25 { 24.0 } else { -24.0 };
            let _ = eng.on_scroll_2d(0.0, dy);
        }
        let scroll_ms = s0.elapsed().as_secs_f64() * 1000.0;
        println!(
            "[perf] 15 interactive taps+DOM-syncs={:.1}ms ({:.2}ms/tap), 50 scrolls={:.2}ms ({:.3}ms/scroll)",
            tap_ms,
            tap_ms / 15.0,
            scroll_ms,
            scroll_ms / 50.0
        );
    }
}
