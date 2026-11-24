//! Built-in Properties, Cache Management, and Browsing History panel
//! (`zipp:settings`, `zipp:properties`, `zipp:history`, `about:settings`, `about:history`).
//!
//! Every mutating form and action link carries the browser's unforgeable per-run
//! chrome token (`__zipp_chrome`), verified by `BrowserNavProvider` before any
//! preference, cache, cookie, or history change is executed.

use crate::{history, settings};

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn mb(bytes: u64) -> String {
    format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn host_monogram(url: &str) -> (String, String) {
    if let Ok(parsed) = url::Url::parse(url)
        && let Some(host) = parsed.host_str()
    {
        let clean = host.strip_prefix("www.").unwrap_or(host);
        let ch = clean
            .chars()
            .find(|c| c.is_ascii_alphanumeric())
            .unwrap_or('W')
            .to_ascii_uppercase()
            .to_string();
        return (ch, clean.to_string());
    }
    ("Z".to_string(), url.to_string())
}

/// Render the interactive Browser Properties & History HTML page.
pub fn render(
    token: &str,
    notice: Option<&str>,
    history_filter: Option<&str>,
    last_page_url: Option<&str>,
    focus_history: bool,
) -> String {
    let cfg = settings::get();
    let incognito = zipp_shell::profile::is_incognito();
    let cache_stats = zipp_shell::cache::stats();
    let cache_disabled = zipp_shell::cache::is_disabled();
    let max_bytes = zipp_shell::cache::max_disk_bytes().max(1);
    let max_mb = max_bytes / (1024 * 1024);
    let cookie_count = zipp_shell::cookies::count();
    let total_history = history::count();
    let entries = history::list(history_filter);

    let disk_pct = ((cache_stats.disk_bytes as f64 / max_bytes as f64) * 100.0).clamp(0.0, 100.0);
    let disk_bar_width = if cache_stats.disk_bytes == 0 {
        0.0
    } else {
        disk_pct.max(2.0)
    };

    let total_lookups = cache_stats.hits + cache_stats.revalidated + cache_stats.misses;
    let cache_eff_val = if total_lookups == 0 {
        format!("{} hits", cache_stats.hits)
    } else {
        let hit_pct = ((cache_stats.hits + cache_stats.revalidated) as f64 / total_lookups as f64) * 100.0;
        format!("{:.0}% hit", hit_pct)
    };

    let filter_val = history_filter.unwrap_or("");
    let notice_html = match notice {
        Some(msg) if !msg.is_empty() => format!(
            r#"<div class="zp-notice">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>
                <span>{}</span>
            </div>"#,
            escape_html(msg)
        ),
        _ => String::new(),
    };

    let mode_banner = if incognito {
        r#"<div class="zp-incognito-banner">
            <div class="zp-incog-icon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg>
            </div>
            <div class="zp-incog-copy">
                <div class="zp-incog-title">Incognito Mode Active</div>
                <div class="zp-incog-desc">Cookies, HTTP cache, localStorage, and IndexedDB stay strictly in memory and are discarded when this window closes. Browsing history is not recorded.</div>
            </div>
        </div>"#
    } else {
        ""
    };

    let use_current_btn = match last_page_url {
        Some(u) if !u.is_empty() && !u.starts_with("zipp:") && !u.starts_with("about:") => {
            let encoded = urlencoding::encode(u);
            format!(
                r#"<a class="zp-preset" href="zipp-settings:set-start?__zipp_chrome={token}&amp;url={encoded}">Use Current Page</a>"#
            )
        }
        _ => String::new(),
    };

    let preset_cls = |url: &str| {
        if cfg.start_page.trim_end_matches('/') == url.trim_end_matches('/') {
            "zp-preset zp-preset-active"
        } else {
            "zp-preset"
        }
    };

    let cache_toggle_label = if cache_disabled { "Enable Cache" } else { "Disable Cache" };
    let cache_status_pill = if cache_disabled {
        r#"<span class="zp-badge zp-badge-warn"><span class="zp-dot zp-dot-warn"></span>Disabled</span>"#
    } else if incognito {
        r#"<span class="zp-badge zp-badge-incog"><span class="zp-dot zp-dot-incog"></span>Memory Only (Incognito)</span>"#
    } else {
        r#"<span class="zp-badge zp-badge-ok"><span class="zp-dot zp-dot-ok"></span>Active · Memory + Disk</span>"#
    };

    let size_buttons: String = [32u64, 64, 128, 256, 512]
        .iter()
        .map(|&sz| {
            let cls = if sz == max_mb { "zp-seg zp-seg-active" } else { "zp-seg" };
            format!(
                r#"<a class="{cls}" href="zipp-settings:cache-limit?__zipp_chrome={token}&amp;mb={sz}">{sz} MB</a>"#
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let mute_label = if cfg.mute_media { "Unmute Audio & Video" } else { "Mute Audio & Video" };
    let mute_badge = if cfg.mute_media {
        r#"<span class="zp-badge zp-badge-warn"><span class="zp-dot zp-dot-warn"></span>Muted</span>"#
    } else {
        r#"<span class="zp-badge zp-badge-ok"><span class="zp-dot zp-dot-ok"></span>Sound On</span>"#
    };

    let history_body = if incognito {
        r#"<div class="zp-empty">
            <div class="zp-empty-icon">
                <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
            </div>
            <div class="zp-empty-title">History paused in Incognito</div>
            <div class="zp-empty-sub">Visits in this private window are never saved to or read from disk.</div>
        </div>"#.to_string()
    } else if entries.is_empty() {
        if filter_val.is_empty() {
            r#"<div class="zp-empty">
                <div class="zp-empty-icon">
                    <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
                </div>
                <div class="zp-empty-title">No browsing history yet</div>
                <div class="zp-empty-sub">Pages you visit in ZIPP Browser will appear here for quick access.</div>
            </div>"#.to_string()
        } else {
            format!(
                r#"<div class="zp-empty">
                    <div class="zp-empty-title">No matches for &ldquo;{}&rdquo;</div>
                    <div class="zp-empty-sub">Try a different keyword or <a class="zp-link" href="zipp:history">show all history</a>.</div>
                </div>"#,
                escape_html(filter_val)
            )
        }
    } else {
        let mut rows = String::with_capacity(entries.len() * 320);
        for item in entries.iter().take(150) {
            let display_title = if item.title.is_empty() {
                item.url.as_str()
            } else {
                item.title.as_str()
            };
            let (mono, _) = host_monogram(&item.url);
            let rel_time = history::format_relative_time(item.visited_at);
            let visits_label = if item.visits == 1 {
                "1 visit".to_string()
            } else {
                format!("{} visits", item.visits)
            };
            rows.push_str(&format!(
                r#"<div class="zp-hist-row">
                    <div class="zp-hist-mono">{mono}</div>
                    <div class="zp-hist-main">
                        <a class="zp-hist-title" href="{url}">{title}</a>
                        <div class="zp-hist-url">{url}</div>
                    </div>
                    <div class="zp-hist-meta">
                        <span class="zp-meta-pill">{visits}</span>
                        <span class="zp-meta-time">{time}</span>
                        <a class="zp-del" title="Remove from history" href="zipp-settings:delete-history?__zipp_chrome={token}&amp;id={id}">&times;</a>
                    </div>
                </div>"#,
                mono = escape_html(&mono),
                url = escape_html(&item.url),
                title = escape_html(display_title),
                visits = visits_label,
                time = escape_html(&rel_time),
                id = item.id,
            ));
        }
        rows
    };

    let title = if focus_history {
        "History — ZIPP Browser"
    } else {
        "Properties — ZIPP Browser"
    };
    let page_heading = if focus_history {
        "Browsing History"
    } else {
        "Browser Properties"
    };
    let page_subtitle = if focus_history {
        "Search, revisit, or clear pages visited in this browser profile."
    } else {
        "Configure your start page, HTTP cache & storage quotas, media audio, and privacy."
    };

    // Official ZIPP obsidian-violet-cyan (normal) and deep amethyst (incognito) color tokens.
    let (bg, rail_bg, card_bg, inset_bg, border, border_strong, accent, accent_soft, primary_grad, primary_hover) = if incognito {
        (
            "#0c0916",
            "#110d1f",
            "#161128",
            "#0f0b1c",
            "#2a2145",
            "#3d3063",
            "#c4b5fd",
            "rgba(139, 92, 246, 0.16)",
            "linear-gradient(180deg, #8b5cf6 0%, #6d28d9 100%)",
            "#7c3aed",
        )
    } else {
        (
            "#0b1020",
            "#10142a",
            "#151933",
            "#0d1124",
            "#262b4d",
            "#3a3f6e",
            "#c4b5fd",
            "rgba(124, 58, 237, 0.18)",
            "linear-gradient(135deg, #7c3aed 0%, #0891b2 100%)",
            "#6d28d9",
        )
    };

    let nav_props_cls = if focus_history { "zp-nav-item" } else { "zp-nav-item zp-nav-active" };
    let nav_hist_cls = if focus_history { "zp-nav-item zp-nav-active" } else { "zp-nav-item" };
    let profile_dot_cls = if incognito { "zp-dot zp-dot-incog" } else { "zp-dot zp-dot-ok" };
    let profile_label = if incognito { "Incognito Session" } else { "Standard Profile" };

    let history_card = format!(
        r#"<section id="history" class="zp-card">
            <div class="zp-card-head">
                <div class="zp-head-left">
                    <div class="zp-icon-box">
                        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
                    </div>
                    <div>
                        <h2>Browsing History</h2>
                        <p class="zp-sub">{total_history} saved {unit} in profile history.</p>
                    </div>
                </div>
                <div class="zp-actions">
                    <a class="zp-btn zp-btn-danger" href="zipp-settings:clear-history?__zipp_chrome={token}">Clear History</a>
                </div>
            </div>
            <form class="zp-search-row" action="zipp-settings:filter-history" method="GET">
                <input type="hidden" name="__zipp_chrome" value="{token}" />
                <div class="zp-input-wrap">
                    <svg class="zp-input-ico" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
                    <input class="zp-input" type="text" name="q" value="{q}" placeholder="Filter history by title or URL..." />
                </div>
                <button class="zp-btn zp-btn-ghost" type="submit">Filter</button>
                {clear_filter}
            </form>
            <div class="zp-hist-list">{history_body}</div>
        </section>"#,
        unit = if total_history == 1 { "page" } else { "pages" },
        q = escape_html(filter_val),
        clear_filter = if filter_val.is_empty() {
            ""
        } else {
            r#"<a class="zp-btn zp-btn-ghost" href="zipp:history">Reset</a>"#
        },
    );

    let settings_cards = format!(
        r#"<section class="zp-card">
            <div class="zp-card-head">
                <div class="zp-head-left">
                    <div class="zp-icon-box">
                        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>
                    </div>
                    <div>
                        <h2>Start Page</h2>
                        <p class="zp-sub">Opened automatically on launch or when pressing Home (Alt+Home).</p>
                    </div>
                </div>
                <a class="zp-btn zp-btn-ghost" href="zipp-nav:home?__zipp_chrome={token}">Open Home</a>
            </div>
            <form class="zp-form-row" action="zipp-settings:save-start" method="GET">
                <input type="hidden" name="__zipp_chrome" value="{token}" />
                <div class="zp-input-wrap">
                    <svg class="zp-input-ico" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></svg>
                    <input class="zp-input" type="text" name="start_page" value="{start_page}" placeholder="https://example.com" />
                </div>
                <button class="zp-btn zp-btn-primary" type="submit">Save Start Page</button>
            </form>
            <div class="zp-quick-row">
                <span class="zp-quick-label">Quick presets</span>
                {use_current_btn}
                <a class="{preset_newtab}" href="zipp-settings:set-start?__zipp_chrome={token}&amp;url=zipp%3Anewtab">New Tab Page (Default)</a>
                <a class="{preset_zipp}" href="zipp-settings:set-start?__zipp_chrome={token}&amp;url=https%3A%2F%2Fwww.zipp.org">ZIPP</a>
                <a class="{preset_softn}" href="zipp-settings:set-start?__zipp_chrome={token}&amp;url=https%3A%2F%2Fsoftn.com">SoftN</a>
                <a class="{preset_search}" href="zipp-settings:set-start?__zipp_chrome={token}&amp;url=https%3A%2F%2Fhtml.duckduckgo.com">DuckDuckGo</a>
                <a class="{preset_props}" href="zipp-settings:set-start?__zipp_chrome={token}&amp;url=zipp%3Asettings">Properties Page</a>
            </div>
        </section>

        <section class="zp-card">
            <div class="zp-card-head">
                <div class="zp-head-left">
                    <div class="zp-icon-box">
                        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14a9 3 0 0 0 18 0V5"/><path d="M3 12a9 3 0 0 0 18 0"/></svg>
                    </div>
                    <div>
                        <h2>Cache &amp; Storage Management</h2>
                        <p class="zp-sub">RFC 9111 HTTP memory + disk cache, cookies, and per-origin storage.</p>
                    </div>
                </div>
                {cache_status_pill}
            </div>
            <div class="zp-meter-box">
                <div class="zp-meter-top">
                    <span class="zp-meter-label">Disk Cache Storage</span>
                    <span class="zp-meter-val">{disk_mb} <span class="zp-muted">/ {max_mb} MB ({disk_pct:.1}%)</span></span>
                </div>
                <div class="zp-meter-track">
                    <div class="zp-meter-fill" style="width:{disk_bar_width:.1}%"></div>
                </div>
            </div>
            <div class="zp-stats-grid">
                <div class="zp-stat">
                    <div class="zp-stat-label">Disk Cache</div>
                    <div class="zp-stat-val">{disk_mb}</div>
                    <div class="zp-stat-sub">{disk_entries} cached files</div>
                </div>
                <div class="zp-stat">
                    <div class="zp-stat-label">Memory Cache</div>
                    <div class="zp-stat-val">{mem_mb}</div>
                    <div class="zp-stat-sub">{mem_entries} hot entries</div>
                </div>
                <div class="zp-stat">
                    <div class="zp-stat-label">Cache Activity</div>
                    <div class="zp-stat-val">{cache_eff_val}</div>
                    <div class="zp-stat-sub">{hits} hit · {reval} 304 · {misses} miss</div>
                </div>
                <div class="zp-stat">
                    <div class="zp-stat-label">Cookies</div>
                    <div class="zp-stat-val">{cookie_count}</div>
                    <div class="zp-stat-sub">{cookie_scope}</div>
                </div>
            </div>
            <div class="zp-limit-row">
                <span class="zp-limit-label">Max Disk Cache Quota</span>
                <div class="zp-seg-group">{size_buttons}</div>
            </div>
            <div class="zp-card-footer">
                <div class="zp-actions-row">
                    <a class="zp-btn zp-btn-ghost" href="zipp-settings:toggle-cache?__zipp_chrome={token}">{cache_toggle_label}</a>
                    <a class="zp-btn zp-btn-primary" href="zipp-settings:clear-cache?__zipp_chrome={token}">Clear HTTP Cache</a>
                </div>
                <a class="zp-btn zp-btn-danger" href="zipp-settings:clear-data?__zipp_chrome={token}">Clear All Site Data</a>
            </div>
        </section>

        <section class="zp-card">
            <div class="zp-card-head">
                <div class="zp-head-left">
                    <div class="zp-icon-box">
                        <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
                    </div>
                    <div>
                        <h2>Privacy, Incognito &amp; Media</h2>
                        <p class="zp-sub">Private browsing isolation and desktop audio &amp; video controls.</p>
                    </div>
                </div>
                {mute_badge}
            </div>
            <div class="zp-setting-rows">
                <div class="zp-setting-row">
                    <div class="zp-setting-info">
                        <div class="zp-setting-title">Incognito Window</div>
                        <div class="zp-setting-desc">Browse with isolated in-memory cookies, cache, localStorage, and IndexedDB — zero files written to disk.</div>
                    </div>
                    <a class="zp-btn zp-btn-incog" href="zipp-nav:incognito?__zipp_chrome={token}">
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg>
                        <span>New Incognito Window</span>
                    </a>
                </div>
                <div class="zp-setting-row zp-setting-row-last">
                    <div class="zp-setting-info">
                        <div class="zp-setting-title">Desktop Sound &amp; Video Audio</div>
                        <div class="zp-setting-desc">Toggle WASAPI Web Audio and Media Foundation &lt;video&gt; sound across this browser profile.</div>
                    </div>
                    <a class="zp-btn zp-btn-ghost" href="zipp-settings:toggle-mute?__zipp_chrome={token}">{mute_label}</a>
                </div>
            </div>
        </section>"#,
        start_page = escape_html(&cfg.start_page),
        preset_newtab = preset_cls("zipp:newtab"),
        preset_zipp = preset_cls("https://www.zipp.org"),
        preset_softn = preset_cls("https://softn.com"),
        preset_search = preset_cls("https://html.duckduckgo.com"),
        preset_props = preset_cls("zipp:settings"),
        disk_mb = mb(cache_stats.disk_bytes),
        disk_entries = cache_stats.disk_entries,
        mem_mb = mb(cache_stats.mem_bytes as u64),
        mem_entries = cache_stats.mem_entries,
        hits = cache_stats.hits,
        reval = cache_stats.revalidated,
        misses = cache_stats.misses,
        cookie_scope = if incognito { "ephemeral only" } else { "persistent + session" },
    );

    let ordered_sections = if focus_history {
        format!("{history_card}\n{settings_cards}")
    } else {
        format!("{settings_cards}\n{history_card}")
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>{title}</title>
<style>
  * {{ box-sizing: border-box; }}
  html, body {{
    margin: 0;
    padding: 0;
    min-height: 100%;
    background: {bg};
    color: #e2e8f0;
    font-family: 'Segoe UI', -apple-system, BlinkMacSystemFont, Arial, sans-serif;
    font-size: 14px;
    line-height: 1.5;
  }}
  .zp-shell {{
    display: flex;
    min-height: 100vh;
    background: linear-gradient(180deg, {rail_bg} 0%, {bg} 260px);
  }}
  .zp-rail {{
    width: 248px;
    flex-shrink: 0;
    background: {rail_bg};
    border-right: 1px solid {border};
    padding: 22px 14px 20px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }}
  .zp-brand {{
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 4px 8px 10px;
    border-bottom: 1px solid {border};
  }}
  .zp-brand-mark {{
    width: 34px;
    height: 34px;
    border-radius: 9px;
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
    flex-shrink: 0;
  }}
  .zp-brand-name {{
    font-size: 15px;
    font-weight: 700;
    color: #f8fafc;
    letter-spacing: -0.01em;
    line-height: 1.2;
  }}
  .zp-brand-profile {{
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: #94a3b8;
    margin-top: 2px;
  }}
  .zp-nav-section {{
    display: flex;
    flex-direction: column;
    gap: 4px;
  }}
  .zp-nav-heading {{
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #64748b;
    padding: 0 10px 4px;
  }}
  .zp-nav-item {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 9px 11px;
    border-radius: 9px;
    color: #cbd5e1;
    text-decoration: none;
    font-size: 13px;
    font-weight: 500;
    border: 1px solid transparent;
  }}
  .zp-nav-item:hover {{
    background: rgba(148, 163, 184, 0.08);
    color: #f8fafc;
  }}
  .zp-nav-active {{
    background: {accent_soft};
    border-color: {border_strong};
    color: #ffffff;
    font-weight: 600;
  }}
  .zp-nav-left {{
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }}
  .zp-nav-count {{
    background: {inset_bg};
    border: 1px solid {border};
    color: #94a3b8;
    font-size: 11px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 999px;
  }}
  .zp-nav-kbd {{
    font-family: Consolas, 'Liberation Mono', monospace;
    font-size: 10px;
    color: #64748b;
    background: {inset_bg};
    border: 1px solid {border};
    padding: 1px 5px;
    border-radius: 5px;
  }}
  .zp-shortcuts {{
    margin-top: 8px;
    padding: 12px;
    background: {inset_bg};
    border: 1px solid {border};
    border-radius: 10px;
  }}
  .zp-shortcuts-title {{
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #64748b;
    margin-bottom: 8px;
  }}
  .zp-sc-row {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    color: #94a3b8;
    padding: 2px 0;
  }}
  .zp-main {{
    flex: 1;
    min-width: 0;
    padding: 28px 34px 56px;
  }}
  .zp-container {{
    max-width: 820px;
    margin: 0 auto;
  }}
  .zp-header {{
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    padding-bottom: 18px;
    margin-bottom: 20px;
    border-bottom: 1px solid {border};
    flex-wrap: wrap;
  }}
  .zp-header h1 {{
    margin: 0;
    font-size: 24px;
    font-weight: 700;
    color: #f8fafc;
    letter-spacing: -0.02em;
  }}
  .zp-header-sub {{
    margin: 4px 0 0;
    color: #94a3b8;
    font-size: 13px;
  }}
  .zp-notice {{
    display: flex;
    align-items: center;
    gap: 10px;
    background: rgba(16, 185, 129, 0.12);
    border: 1px solid rgba(52, 211, 153, 0.35);
    color: #a7f3d0;
    padding: 11px 14px;
    border-radius: 10px;
    margin-bottom: 18px;
    font-size: 13px;
    font-weight: 500;
  }}
  .zp-incognito-banner {{
    display: flex;
    align-items: center;
    gap: 14px;
    background: linear-gradient(90deg, rgba(109, 40, 217, 0.24) 0%, rgba(76, 29, 149, 0.14) 100%);
    border: 1px solid rgba(167, 139, 250, 0.38);
    color: #ede9fe;
    padding: 14px 16px;
    border-radius: 12px;
    margin-bottom: 18px;
  }}
  .zp-incog-icon {{
    width: 36px;
    height: 36px;
    border-radius: 10px;
    background: rgba(139, 92, 246, 0.28);
    color: #ddd6fe;
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }}
  .zp-incog-title {{
    font-size: 14px;
    font-weight: 700;
    color: #f5f3ff;
  }}
  .zp-incog-desc {{
    font-size: 12px;
    color: #c4b5fd;
    margin-top: 2px;
  }}
  .zp-card {{
    background: {card_bg};
    border: 1px solid {border};
    border-radius: 14px;
    padding: 20px 22px;
    margin-bottom: 18px;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.22);
  }}
  .zp-card-head {{
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 14px;
    margin-bottom: 16px;
  }}
  .zp-head-left {{
    display: flex;
    align-items: flex-start;
    gap: 12px;
  }}
  .zp-icon-box {{
    width: 34px;
    height: 34px;
    border-radius: 9px;
    background: {accent_soft};
    border: 1px solid {border_strong};
    color: {accent};
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    margin-top: 1px;
  }}
  .zp-card h2 {{
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    color: #f8fafc;
    letter-spacing: -0.01em;
  }}
  .zp-sub {{
    margin: 2px 0 0;
    color: #94a3b8;
    font-size: 13px;
  }}
  .zp-form-row, .zp-search-row {{
    display: flex;
    gap: 9px;
    align-items: center;
    margin-bottom: 12px;
  }}
  .zp-input-wrap {{
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 38px;
    padding: 0 12px;
    background: {inset_bg};
    border: 1px solid {border_strong};
    border-radius: 9px;
  }}
  .zp-input-ico {{
    color: #64748b;
    flex-shrink: 0;
  }}
  .zp-input {{
    flex: 1;
    min-width: 0;
    height: 34px;
    padding: 0;
    background: transparent;
    color: #f8fafc;
    border: 0;
    font-size: 13px;
    outline: none;
  }}
  .zp-quick-row {{
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
    align-items: center;
  }}
  .zp-quick-label {{
    font-size: 12px;
    color: #64748b;
    margin-right: 4px;
  }}
  .zp-preset {{
    display: inline-flex;
    align-items: center;
    height: 28px;
    padding: 0 11px;
    border-radius: 999px;
    background: {inset_bg};
    border: 1px solid {border};
    color: #cbd5e1;
    font-size: 12px;
    font-weight: 500;
    text-decoration: none;
  }}
  .zp-preset:hover {{
    border-color: {border_strong};
    color: #ffffff;
  }}
  .zp-preset-active {{
    background: {accent_soft};
    border-color: {accent};
    color: #ffffff;
    font-weight: 600;
  }}
  .zp-actions-row, .zp-actions {{
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: center;
  }}
  .zp-card-footer {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
    padding-top: 14px;
    margin-top: 14px;
    border-top: 1px solid {border};
  }}
  .zp-btn {{
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 7px;
    height: 36px;
    padding: 0 14px;
    border-radius: 9px;
    font-size: 13px;
    font-weight: 600;
    text-decoration: none;
    cursor: pointer;
    border: 1px solid transparent;
    white-space: nowrap;
  }}
  .zp-btn-primary {{
    background: {primary_grad};
    color: #ffffff;
    border-color: rgba(255, 255, 255, 0.12);
    box-shadow: 0 2px 6px rgba(0, 0, 0, 0.25);
  }}
  .zp-btn-primary:hover {{
    background: {primary_hover};
  }}
  .zp-btn-ghost {{
    background: {inset_bg};
    color: #e2e8f0;
    border-color: {border_strong};
  }}
  .zp-btn-ghost:hover {{
    background: rgba(148, 163, 184, 0.12);
    color: #ffffff;
  }}
  .zp-btn-danger {{
    background: rgba(239, 68, 68, 0.12);
    color: #fca5a5;
    border-color: rgba(239, 68, 68, 0.34);
  }}
  .zp-btn-danger:hover {{
    background: rgba(220, 38, 38, 0.28);
    color: #fee2e2;
  }}
  .zp-btn-incog {{
    background: linear-gradient(180deg, #8b5cf6 0%, #6d28d9 100%);
    color: #ffffff;
    border-color: rgba(255, 255, 255, 0.14);
  }}
  .zp-btn-incog:hover {{
    background: #7c3aed;
  }}
  .zp-meter-box {{
    background: {inset_bg};
    border: 1px solid {border};
    border-radius: 10px;
    padding: 11px 14px;
    margin-bottom: 12px;
  }}
  .zp-meter-top {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 12px;
    margin-bottom: 8px;
  }}
  .zp-meter-label {{
    color: #cbd5e1;
    font-weight: 600;
  }}
  .zp-meter-val {{
    color: #f8fafc;
    font-weight: 600;
  }}
  .zp-muted {{ color: #64748b; font-weight: 500; }}
  .zp-meter-track {{
    height: 7px;
    border-radius: 999px;
    background: rgba(148, 163, 184, 0.12);
    overflow: hidden;
  }}
  .zp-meter-fill {{
    height: 100%;
    border-radius: 999px;
    background: {primary_grad};
  }}
  .zp-stats-grid {{
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
    margin-bottom: 14px;
  }}
  .zp-stat {{
    background: {inset_bg};
    border: 1px solid {border};
    border-radius: 10px;
    padding: 11px 13px;
  }}
  .zp-stat-label {{
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #64748b;
  }}
  .zp-stat-val {{
    font-size: 18px;
    font-weight: 700;
    color: #f8fafc;
    margin: 3px 0 2px;
    letter-spacing: -0.01em;
  }}
  .zp-stat-sub {{
    font-size: 11px;
    color: #94a3b8;
  }}
  .zp-limit-row {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }}
  .zp-limit-label {{
    color: #cbd5e1;
    font-size: 13px;
    font-weight: 500;
  }}
  .zp-seg-group {{
    display: inline-flex;
    background: {inset_bg};
    border: 1px solid {border};
    border-radius: 9px;
    padding: 3px;
    gap: 2px;
  }}
  .zp-seg {{
    padding: 5px 11px;
    border-radius: 6px;
    color: #94a3b8;
    text-decoration: none;
    font-size: 12px;
    font-weight: 600;
  }}
  .zp-seg:hover {{
    color: #f8fafc;
  }}
  .zp-seg-active {{
    background: {primary_grad};
    color: #ffffff;
  }}
  .zp-setting-rows {{
    display: flex;
    flex-direction: column;
  }}
  .zp-setting-row {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 13px 0;
    border-top: 1px solid {border};
    flex-wrap: wrap;
  }}
  .zp-setting-row-last {{
    padding-bottom: 2px;
  }}
  .zp-setting-info {{
    flex: 1;
    min-width: 240px;
  }}
  .zp-setting-title {{
    font-size: 14px;
    font-weight: 600;
    color: #f1f5f9;
  }}
  .zp-setting-desc {{
    font-size: 12px;
    color: #94a3b8;
    margin-top: 2px;
  }}
  .zp-badge {{
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }}
  .zp-dot {{
    width: 7px;
    height: 7px;
    border-radius: 999px;
    display: inline-block;
  }}
  .zp-dot-ok {{ background: #34d399; }}
  .zp-dot-warn {{ background: #fb923c; }}
  .zp-dot-incog {{ background: #a78bfa; }}
  .zp-badge-ok {{ background: rgba(16, 185, 129, 0.14); border: 1px solid rgba(52, 211, 153, 0.3); color: #6ee7b7; }}
  .zp-badge-warn {{ background: rgba(249, 115, 22, 0.14); border: 1px solid rgba(251, 146, 60, 0.3); color: #fdba74; }}
  .zp-badge-incog {{ background: rgba(139, 92, 246, 0.18); border: 1px solid rgba(167, 139, 250, 0.35); color: #ddd6fe; }}
  .zp-hist-list {{
    display: flex;
    flex-direction: column;
    gap: 7px;
  }}
  .zp-hist-row {{
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    background: {inset_bg};
    border: 1px solid {border};
    border-radius: 10px;
  }}
  .zp-hist-row:hover {{
    border-color: {border_strong};
  }}
  .zp-hist-mono {{
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: {accent_soft};
    border: 1px solid {border_strong};
    color: {accent};
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 12px;
    font-weight: 700;
    flex-shrink: 0;
  }}
  .zp-hist-main {{
    min-width: 0;
    flex: 1;
  }}
  .zp-hist-title {{
    color: #f1f5f9;
    text-decoration: none;
    font-size: 13px;
    font-weight: 600;
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .zp-hist-title:hover {{
    color: {accent};
  }}
  .zp-hist-url {{
    color: #64748b;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .zp-hist-meta {{
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }}
  .zp-meta-pill {{
    background: {card_bg};
    border: 1px solid {border};
    color: #94a3b8;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: 999px;
  }}
  .zp-meta-time {{
    color: #64748b;
    font-size: 12px;
    min-width: 58px;
    text-align: right;
  }}
  .zp-del {{
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    background: rgba(239, 68, 68, 0.12);
    border: 1px solid rgba(239, 68, 68, 0.24);
    color: #fca5a5;
    text-decoration: none;
    font-weight: 700;
    font-size: 15px;
  }}
  .zp-del:hover {{
    background: rgba(220, 38, 38, 0.28);
    color: #ffffff;
  }}
  .zp-empty {{
    padding: 28px 20px;
    text-align: center;
    background: {inset_bg};
    border: 1px dashed {border_strong};
    border-radius: 10px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }}
  .zp-empty-icon {{
    width: 42px;
    height: 42px;
    border-radius: 12px;
    background: {accent_soft};
    color: {accent};
    display: flex;
    align-items: center;
    justify-content: center;
    margin-bottom: 4px;
  }}
  .zp-empty-title {{
    font-size: 14px;
    font-weight: 600;
    color: #e2e8f0;
  }}
  .zp-empty-sub {{
    font-size: 12px;
    color: #94a3b8;
  }}
  .zp-link {{ color: {accent}; text-decoration: none; font-weight: 600; }}
  @media (max-width: 780px) {{
    .zp-shell {{ flex-direction: column; }}
    .zp-rail {{
      width: 100%;
      border-right: 0;
      border-bottom: 1px solid {border};
      padding: 14px 16px;
      gap: 12px;
    }}
    .zp-shortcuts {{ display: none; }}
    .zp-main {{ padding: 18px 14px 40px; }}
    .zp-stats-grid {{ grid-template-columns: repeat(2, 1fr); }}
  }}
</style>
</head>
<body>
  <div class="zp-shell">
    <aside class="zp-rail">
      <div class="zp-brand">
        <div class="zp-brand-mark">
          <svg width="34" height="34" viewBox="0 0 256 256" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="zb_bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0b1020"/><stop offset="0.55" stop-color="#24124d"/><stop offset="1" stop-color="#083b4c"/></linearGradient><linearGradient id="zb_bolt" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#c4b5fd"/><stop offset="0.5" stop-color="#7c3aed"/><stop offset="1" stop-color="#22d3ee"/></linearGradient></defs><rect width="256" height="256" rx="56" fill="url(#zb_bg)"/><path d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)" fill="url(#zb_bolt)"/></svg>
        </div>
        <div>
          <div class="zp-brand-name">ZIPP Browser</div>
          <div class="zp-brand-profile"><span class="{profile_dot_cls}"></span><span>{profile_label}</span></div>
        </div>
      </div>

      <nav class="zp-nav-section">
        <div class="zp-nav-heading">Settings</div>
        <a class="{nav_props_cls}" href="zipp:settings">
          <span class="zp-nav-left">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.604.852.997 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>
            <span>Properties</span>
          </span>
        </a>
        <a class="{nav_hist_cls}" href="zipp:history">
          <span class="zp-nav-left">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
            <span>History</span>
          </span>
          <span class="zp-nav-count">{total_history}</span>
        </a>
      </nav>

      <div class="zp-nav-section">
        <div class="zp-nav-heading">Quick Actions</div>
        <a class="zp-nav-item" href="zipp-nav:incognito?__zipp_chrome={token}">
          <span class="zp-nav-left">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg>
            <span>Incognito</span>
          </span>
          <span class="zp-nav-kbd">Ctrl+Shift+N</span>
        </a>
        <a class="zp-nav-item" href="zipp-nav:home?__zipp_chrome={token}">
          <span class="zp-nav-left">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg>
            <span>Home Page</span>
          </span>
          <span class="zp-nav-kbd">Alt+Home</span>
        </a>
        <a class="zp-nav-item" href="zipp-nav:back?__zipp_chrome={token}">
          <span class="zp-nav-left">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>Back to Page</span>
          </span>
        </a>
      </div>

      <div class="zp-shortcuts">
        <div class="zp-shortcuts-title">Shortcuts</div>
        <div class="zp-sc-row"><span>New Tab</span><span class="zp-nav-kbd">Ctrl+T</span></div>
        <div class="zp-sc-row"><span>Close Tab</span><span class="zp-nav-kbd">Ctrl+W</span></div>
        <div class="zp-sc-row"><span>Reopen Tab</span><span class="zp-nav-kbd">Ctrl+Shift+T</span></div>
        <div class="zp-sc-row"><span>Switch Tabs</span><span class="zp-nav-kbd">Ctrl+Tab</span></div>
        <div class="zp-sc-row"><span>Tab 1&ndash;9</span><span class="zp-nav-kbd">Ctrl+1..9</span></div>
        <div class="zp-sc-row"><span>Properties</span><span class="zp-nav-kbd">Ctrl+,</span></div>
        <div class="zp-sc-row"><span>History</span><span class="zp-nav-kbd">Ctrl+H</span></div>
        <div class="zp-sc-row"><span>Mobile View</span><span class="zp-nav-kbd">Ctrl+Shift+M</span></div>
        <div class="zp-sc-row"><span>Inspect</span><span class="zp-nav-kbd">Ctrl+Shift+I</span></div>
        <div class="zp-sc-row"><span>Pick Element</span><span class="zp-nav-kbd">Ctrl+Shift+C</span></div>
        <div class="zp-sc-row"><span>View Source</span><span class="zp-nav-kbd">Ctrl+U</span></div>
        <div class="zp-sc-row"><span>Dev Console</span><span class="zp-nav-kbd">F12</span></div>
      </div>
    </aside>

    <main class="zp-main">
      <div class="zp-container">
        <header class="zp-header">
          <div>
            <h1>{page_heading}</h1>
            <p class="zp-header-sub">{page_subtitle}</p>
          </div>
          <a class="zp-btn zp-btn-ghost" href="zipp-nav:back?__zipp_chrome={token}">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>
            <span>Back</span>
          </a>
        </header>
        {mode_banner}
        {notice_html}
        {ordered_sections}
      </div>
    </main>
  </div>
</body>
</html>"##
    )
}

/// Render the built-in New Tab start page (`zipp:newtab` / `about:newtab`).
pub fn render_newtab(token: &str) -> String {
    let cfg = settings::get();
    let incognito = zipp_shell::profile::is_incognito();
    let (home_url, home_title, home_display) = if cfg.start_page == "zipp:newtab" {
        (
            "zipp:settings".to_string(),
            "Start Page",
            "Choose your home page".to_string(),
        )
    } else if cfg.start_page.starts_with("zipp:") {
        (cfg.start_page.clone(), "Home", "Start Page".to_string())
    } else {
        (
            cfg.start_page.clone(),
            "Home",
            host_monogram(&cfg.start_page).1,
        )
    };

    let (bg, glow, card_bg, inset_bg, border, border_strong, accent, accent_soft, primary_grad, badge_html, subtitle) = if incognito {
        (
            "#0c0916",
            "radial-gradient(ellipse at 50% 0%, rgba(139, 92, 246, 0.22) 0%, rgba(12, 9, 22, 0) 65%)",
            "#161128",
            "#100c1e",
            "#2a2145",
            "#3f3168",
            "#c4b5fd",
            "rgba(139, 92, 246, 0.18)",
            "linear-gradient(180deg, #8b5cf6 0%, #6d28d9 100%)",
            r#"<span class="nt-pill"><svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg>Incognito Window</span>"#,
            "Private session — cookies, cache, and storage stay strictly in memory and vanish when closed.",
        )
    } else {
        (
            "#0b1020",
            "radial-gradient(ellipse at 50% 0%, rgba(124, 58, 237, 0.24) 0%, rgba(8, 59, 76, 0.14) 38%, rgba(11, 16, 32, 0) 70%)",
            "#151933",
            "#0d1124",
            "#262b4d",
            "#3a3f6e",
            "#c4b5fd",
            "rgba(124, 58, 237, 0.18)",
            "linear-gradient(135deg, #7c3aed 0%, #0891b2 100%)",
            r#"<span class="nt-pill">Part of the ZIPP Project &middot; Native Web &amp; SoftN Browser</span>"#,
            "Powered by ZIPP — search the web, launch a SoftN app, or jump into a site below.",
        )
    };

    let recent_section = if incognito {
        r#"<section class="nt-section">
            <div class="nt-incog-card">
                <div class="nt-incog-ico">
                    <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/></svg>
                </div>
                <div>
                    <div class="nt-incog-title">You&rsquo;re browsing privately</div>
                    <div class="nt-incog-desc">History is paused and nothing from this window is written to disk.</div>
                </div>
            </div>
        </section>"#.to_string()
    } else {
        let entries = history::list(None);
        let recent: Vec<_> = entries
            .into_iter()
            .filter(|e| !e.url.starts_with("zipp:") && !e.url.starts_with("about:"))
            .take(6)
            .collect();
        if recent.is_empty() {
            String::new()
        } else {
            let mut items = String::with_capacity(recent.len() * 260);
            for item in recent {
                let (mono, host) = host_monogram(&item.url);
                let raw_label = if item.title.is_empty() { host.as_str() } else { item.title.as_str() };
                let label: String = if raw_label.chars().count() > 36 {
                    let mut s: String = raw_label.chars().take(35).collect();
                    s.push('…');
                    s
                } else {
                    raw_label.to_string()
                };
                items.push_str(&format!(
                    r#"<a class="nt-recent-card" href="{url}">
                        <span class="nt-recent-mono">{mono}</span>
                        <span class="nt-recent-copy">
                            <span class="nt-recent-title">{label}</span>
                            <span class="nt-recent-host">{host}</span>
                        </span>
                    </a>"#,
                    url = escape_html(&item.url),
                    mono = escape_html(&mono),
                    label = escape_html(&label),
                    host = escape_html(&host),
                ));
            }
            format!(
                r#"<section class="nt-section">
                    <div class="nt-sec-head">
                        <span class="nt-sec-title">Recently Visited</span>
                        <a class="nt-sec-link" href="zipp:history">All history &rarr;</a>
                    </div>
                    <div class="nt-recent-grid">{items}</div>
                </section>"#
            )
        }
    };

    let home_href = escape_html(&home_url);
    let home_label = escape_html(&home_display);

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>New Tab — ZIPP Browser</title>
<style>
  * {{ box-sizing: border-box; }}
  html, body {{
    margin: 0;
    padding: 0;
    min-height: 100%;
    background: {bg};
    color: #e2e8f0;
    font-family: 'Segoe UI', -apple-system, BlinkMacSystemFont, Arial, sans-serif;
    font-size: 14px;
    line-height: 1.5;
  }}
  .nt-page {{
    min-height: 100vh;
    background: {glow};
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 52px 20px 44px;
  }}
  .nt-wrap {{
    width: 100%;
    max-width: 740px;
    display: flex;
    flex-direction: column;
    gap: 28px;
  }}
  .nt-hero {{
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    gap: 12px;
  }}
  .nt-brand-row {{
    display: flex;
    align-items: center;
    gap: 12px;
  }}
  .nt-logo {{
    width: 44px;
    height: 44px;
    border-radius: 11px;
    background: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 8px 24px rgba(124, 58, 237, 0.32);
  }}
  .nt-title {{
    margin: 0;
    font-size: 28px;
    font-weight: 700;
    letter-spacing: -0.025em;
    color: #f8fafc;
  }}
  .nt-pill {{
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 11px;
    border-radius: 999px;
    background: {accent_soft};
    border: 1px solid {border_strong};
    color: {accent};
    font-size: 12px;
    font-weight: 600;
  }}
  .nt-sub {{
    margin: 0;
    font-size: 13.5px;
    color: #94a3b8;
    max-width: 520px;
  }}
  .nt-search {{
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px 6px 16px;
    background: {card_bg};
    border: 1px solid {border_strong};
    border-radius: 14px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.28);
  }}
  .nt-search-ico {{
    color: {accent};
    flex-shrink: 0;
  }}
  .nt-search-input {{
    flex: 1;
    min-width: 0;
    height: 36px;
    border: 0;
    outline: none;
    background: transparent;
    color: #f8fafc;
    font-family: inherit;
    font-size: 14.5px;
  }}
  .nt-search-btn {{
    height: 36px;
    padding: 0 18px;
    border-radius: 10px;
    border: 0;
    background: {primary_grad};
    color: #ffffff;
    font-family: inherit;
    font-size: 13.5px;
    font-weight: 600;
    cursor: pointer;
    flex-shrink: 0;
  }}
  .nt-section {{
    display: flex;
    flex-direction: column;
    gap: 12px;
  }}
  .nt-sec-head {{
    display: flex;
    align-items: center;
    justify-content: space-between;
  }}
  .nt-sec-title {{
    font-size: 11.5px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #64748b;
  }}
  .nt-sec-link {{
    font-size: 12.5px;
    font-weight: 600;
    color: {accent};
    text-decoration: none;
  }}
  .nt-tiles {{
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }}
  .nt-tile {{
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 13px 14px;
    border-radius: 12px;
    background: {card_bg};
    border: 1px solid {border};
    text-decoration: none;
    color: #e2e8f0;
  }}
  .nt-tile:hover {{
    border-color: {border_strong};
    background: {inset_bg};
  }}
  .nt-tile-ico {{
    width: 34px;
    height: 34px;
    border-radius: 10px;
    background: {accent_soft};
    color: {accent};
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }}
  .nt-tile-text {{
    min-width: 0;
    display: flex;
    flex-direction: column;
  }}
  .nt-tile-name {{
    font-size: 13.5px;
    font-weight: 600;
    color: #f8fafc;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .nt-tile-sub {{
    font-size: 11.5px;
    color: #94a3b8;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .nt-recent-grid {{
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 9px;
  }}
  .nt-recent-card {{
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 11px;
    background: {card_bg};
    border: 1px solid {border};
    text-decoration: none;
    color: #e2e8f0;
    min-width: 0;
  }}
  .nt-recent-card:hover {{
    border-color: {border_strong};
  }}
  .nt-recent-mono {{
    width: 30px;
    height: 30px;
    border-radius: 8px;
    background: {inset_bg};
    border: 1px solid {border};
    color: {accent};
    display: flex;
    align-items: center;
    justify-content: center;
    font-weight: 700;
    font-size: 12.5px;
    flex-shrink: 0;
  }}
  .nt-recent-copy {{
    flex: 1;
    min-width: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }}
  .nt-recent-title {{
    font-size: 13px;
    font-weight: 600;
    color: #f1f5f9;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .nt-recent-host {{
    font-size: 11.5px;
    color: #64748b;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }}
  .nt-incog-card {{
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 16px 18px;
    border-radius: 12px;
    background: {card_bg};
    border: 1px solid {border_strong};
  }}
  .nt-incog-ico {{
    width: 38px;
    height: 38px;
    border-radius: 10px;
    background: {accent_soft};
    color: {accent};
    display: flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
  }}
  .nt-incog-title {{
    font-size: 14px;
    font-weight: 700;
    color: #f5f3ff;
  }}
  .nt-incog-desc {{
    font-size: 12.5px;
    color: #c4b5fd;
  }}
  .nt-keys {{
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
    padding-top: 8px;
  }}
  .nt-key-pill {{
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 999px;
    background: {inset_bg};
    border: 1px solid {border};
    color: #94a3b8;
    font-size: 11.5px;
  }}
  .nt-kbd {{
    font-family: Consolas, 'Liberation Mono', monospace;
    font-size: 11px;
    font-weight: 700;
    color: #e2e8f0;
  }}
  @media (max-width: 640px) {{
    .nt-page {{ padding: 28px 14px 32px; }}
    .nt-tiles {{ grid-template-columns: repeat(2, minmax(0, 1fr)); }}
    .nt-recent-grid {{ grid-template-columns: 1fr; }}
  }}
</style>
</head>
<body>
  <main class="nt-page">
    <div class="nt-wrap">
      <header class="nt-hero">
        <div class="nt-brand-row">
          <div class="nt-logo">
            <svg width="44" height="44" viewBox="0 0 256 256" xmlns="http://www.w3.org/2000/svg"><defs><linearGradient id="nt_bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0b1020"/><stop offset="0.55" stop-color="#24124d"/><stop offset="1" stop-color="#083b4c"/></linearGradient><linearGradient id="nt_bolt" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#c4b5fd"/><stop offset="0.5" stop-color="#7c3aed"/><stop offset="1" stop-color="#22d3ee"/></linearGradient></defs><rect width="256" height="256" rx="56" fill="url(#nt_bg)" stroke="#a78bfa" stroke-opacity="0.35" stroke-width="6"/><path d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)" fill="url(#nt_bolt)"/></svg>
          </div>
          <h1 class="nt-title">ZIPP Browser</h1>
        </div>
        {badge_html}
        <p class="nt-sub">{subtitle}</p>
      </header>

      <form class="nt-search" action="zipp:newtab" method="GET">
        <input type="hidden" name="__zipp_chrome" value="{token}" />
        <svg class="nt-search-ico" width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
        <input class="nt-search-input" type="text" name="__nav_url" placeholder="Search the web or enter a URL..." />
        <button class="nt-search-btn" type="submit">Go</button>
      </form>

      <section class="nt-section">
        <div class="nt-sec-head">
          <span class="nt-sec-title">Quick Access</span>
        </div>
        <div class="nt-tiles">
          <a class="nt-tile" href="{home_href}">
            <span class="nt-tile-ico"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><polyline points="9 22 9 12 15 12 15 22"/></svg></span>
            <span class="nt-tile-text"><span class="nt-tile-name">{home_title}</span><span class="nt-tile-sub">{home_label}</span></span>
          </a>
          <a class="nt-tile" href="https://softn.com">
            <span class="nt-tile-ico"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polygon points="12 2 2 7 12 12 22 7 12 2"/><polyline points="2 17 12 22 22 17"/><polyline points="2 12 12 17 22 12"/></svg></span>
            <span class="nt-tile-text"><span class="nt-tile-name">SoftN</span><span class="nt-tile-sub">softn.com</span></span>
          </a>
          <a class="nt-tile" href="https://www.zipp.org">
            <span class="nt-tile-ico"><svg width="16" height="16" viewBox="0 0 256 256" xmlns="http://www.w3.org/2000/svg"><path d="M92 0 0 122h58l-25 86L142 69h-62z" transform="translate(57 24)" fill="currentColor"/></svg></span>
            <span class="nt-tile-text"><span class="nt-tile-name">ZIPP Project</span><span class="nt-tile-sub">zipp.org</span></span>
          </a>
          <a class="nt-tile" href="zipp:settings">
            <span class="nt-tile-ico"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.604.852.997 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg></span>
            <span class="nt-tile-text"><span class="nt-tile-name">Properties</span><span class="nt-tile-sub">Cache &amp; start page</span></span>
          </a>
          <a class="nt-tile" href="zipp:history">
            <span class="nt-tile-ico"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg></span>
            <span class="nt-tile-text"><span class="nt-tile-name">History</span><span class="nt-tile-sub">Visited pages</span></span>
          </a>
          <a class="nt-tile" href="zipp-nav:incognito?__zipp_chrome={token}">
            <span class="nt-tile-ico"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"/><circle cx="12" cy="12" r="3"/><path d="m3 3 18 18"/></svg></span>
            <span class="nt-tile-text"><span class="nt-tile-name">Incognito</span><span class="nt-tile-sub">Private window</span></span>
          </a>
        </div>
      </section>

      {recent_section}

      <footer class="nt-keys">
        <span class="nt-key-pill"><span class="nt-kbd">Ctrl+T</span> New Tab</span>
        <span class="nt-key-pill"><span class="nt-kbd">Ctrl+W</span> Close Tab</span>
        <span class="nt-key-pill"><span class="nt-kbd">Ctrl+Tab</span> Switch Tab</span>
        <span class="nt-key-pill"><span class="nt-kbd">Ctrl+Shift+T</span> Reopen Closed</span>
        <span class="nt-key-pill"><span class="nt-kbd">Ctrl+L</span> Address Bar</span>
      </footer>
    </div>
  </main>
</body>
</html>"##
    )
}
