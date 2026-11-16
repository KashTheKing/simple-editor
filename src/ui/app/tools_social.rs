//! ---- ws:social-guides ----
//! MCP tools for the social guides and profiles: guides.list / guides.set / guides.preview and
//! profile.get / profile.set / profile.fetch. None of them touches the project; `profile.fetch` is held by
//! the Assistant guard (`mcp::tools::leaves_project`) because it reads a web page.

use super::thumbs::base64;
use super::tools_helpers::*;
use super::*;
use crate::mcp::tools::{ToolDef, ToolKind, ToolOutcome};
use crate::settings::Settings;
use crate::ui::social::device::Sheet;
use crate::ui::social::paint::GuideView;
use crate::ui::social::profile::{self, SampleComment, SocialProfile};
use crate::ui::social::{self, raster, Mode, ViewMode};

/// `render.frame` / `render.contact_sheet` with the guide drawn in: when `guide` is true, or the setting
/// "Draw in render.frame and contact sheets" is on. A device preview comes back as the device picture.
pub(super) fn decorate(app: &App, f: Arc<Frame>, t: f64, args: &Value) -> Arc<Frame> {
    if !arg_bool(args, "guide").unwrap_or(app.settings.guide_render) || app.settings.guide.is_none() {
        return f;
    }
    let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
    let Some(g) = GuideView::new(&app.settings, &app.project.name, aspect, t, app.project.duration()) else { return f };
    Arc::new(match g.mode {
        Mode::Device => raster::render_device(Some(&f), &g, None, f.width.max(320)),
        _ => raster::render_overlay((*f).clone(), &g),
    })
}

/// `state`: a sheet (comments, desc, share, more, profile, sound), a desktop player view (theatre, fullscreen),
/// or default (nothing open, default view).
fn state_arg(args: &Value) -> Result<Option<Sheet>, String> {
    match arg_str(args, "state") {
        None | Some("") | Some("default") | Some("closed") | Some("theatre") | Some("fullscreen") | Some("full") => Ok(None),
        Some(s) => Sheet::from_id(s).map(Some).ok_or_else(|| format!("state: default, theatre, fullscreen, comments, desc, share, more, profile or sound (got '{s}')")),
    }
}

/// The player view a `state` names (default resets it); None when `state` is a sheet or absent.
fn view_arg(args: &Value) -> Option<ViewMode> {
    match arg_str(args, "state")? {
        "default" | "closed" => Some(ViewMode::Default),
        s => ViewMode::from_id(s),
    }
}

fn profile_json(p: &SocialProfile) -> Value {
    json!({
        "id": p.id, "network": p.net, "name": p.name, "handle": p.handle, "display": p.display, "verified": p.verified,
        "followers": p.followers, "caption": p.caption, "tags": p.tags, "sound": p.sound, "likes": p.likes,
        "comments": p.comments, "shares": p.shares, "saves": p.saves, "views": p.views, "link": p.link,
        "has_avatar": !p.avatar.is_empty(), "note": p.note,
        "comment_list": p.comment_list.iter().map(|c| json!({"name": c.name, "text": c.text, "likes": c.likes, "when": c.when})).collect::<Vec<_>>(),
    })
}

fn fractions(r: [f32; 4], rf: [f32; 2]) -> Value {
    let q = |v: f32| (v * 1000.0).round() / 1000.0;
    json!([q(r[0] / rf[0]), q(r[1] / rf[1]), q(r[2] / rf[0]), q(r[3] / rf[1])])
}

fn current(app: &App) -> Value {
    let s = &app.settings;
    let net = s.guide.as_deref().and_then(|g| social::data().platform(g).map(|p| p.net.clone()));
    json!({
        "platform": s.guide, "mode": s.guide_mode, "device": s.guide_device, "worst_case": s.guide_worst,
        "keep_aspect": s.guides_keep_aspect, "draw_in_render": s.guide_render,
        "profile": net.and_then(|n| s.active_profile(&n).map(|p| p.id.clone())),
        "state": app.preview.social.want.map_or(app.settings.guide_view.as_str(), |x| x.id()),
        "player_view": app.settings.guide_view,
    })
}

fn set_profile_fields(p: &mut SocialProfile, f: &Value) -> Result<(), String> {
    let o = f.as_object().ok_or("fields must be an object")?;
    for (k, v) in o {
        let text = |v: &Value| v.as_str().map(str::to_string).ok_or(format!("{k}: expected a string"));
        match k.as_str() {
            "name" => p.name = text(v)?,
            "handle" => p.handle = text(v)?.trim_start_matches('@').to_string(),
            "display" => p.display = text(v)?,
            "followers" => p.followers = text(v)?,
            "caption" => p.caption = text(v)?,
            "tags" => p.tags = text(v)?,
            "sound" => p.sound = text(v)?,
            "likes" => p.likes = text(v)?,
            "comments" => p.comments = text(v)?,
            "shares" => p.shares = text(v)?,
            "saves" => p.saves = text(v)?,
            "views" => p.views = text(v)?,
            "link" => p.link = text(v)?,
            "verified" => p.verified = v.as_bool().ok_or("verified: expected true or false")?,
            "comment_list" => {
                p.comment_list = v
                    .as_array()
                    .ok_or("comment_list: expected an array")?
                    .iter()
                    .map(|c| {
                        let g = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        SampleComment { name: g("name"), text: g("text"), likes: g("likes"), when: g("when") }
                    })
                    .collect()
            }
            other => return Err(format!("unknown field '{other}'")),
        }
    }
    Ok(())
}

pub const TOOLS: &[ToolDef] = &[
    ToolDef {
        name: "guides.list",
        desc: "The social guides: every platform (id, network, aspect, 'verified on' date, where it was measured), its zones as fractions of the frame, how comments/description/share/more behave, the devices, and the current setting. Draw one with guides.set; picture it with guides.preview.",
        args: &[],
        kind: ToolKind::Read,
        run: |app, _| {
            let d = social::data();
            let platforms: Vec<Value> = d
                .platforms
                .iter()
                .map(|p| {
                    let lay: serde_json::Map<String, Value> = p
                        .lay
                        .iter()
                        .map(|(k, l)| {
                            let zones: Vec<Value> = l.zones.iter().map(|z| json!({"kind": match z.k.as_str() { "d" => "danger", "c" => "caution", "s" => "safe", _ => "crop" }, "rect": fractions(z.r, l.refsz), "label": z.l})).collect();
                            (k.clone(), json!({"zones": zones, "fit": l.fit, "reference_dp": l.refsz, "elements": l.ui.len()}))
                        })
                        .collect();
                    let sh = |s: &social::Sheet| json!({"behaviour": s.m, "sheet_top": s.sh, "ms": s.ms, "measured": s.src != "est"});
                    json!({"id": p.id, "name": p.n, "network": p.net, "page": p.pg, "aspect": p.a, "verified": p.ver, "source": p.src, "layouts": lay,
                           "sheets": {"comments": sh(&p.st.comments), "desc": sh(&p.st.desc), "share": sh(&p.st.share), "more": sh(&p.st.more)}})
                })
                .collect();
            let devices: Vec<Value> = d.devices.iter().map(|v| json!({"id": v.id, "name": v.n, "kind": v.k, "size_dp": [v.w, v.h]})).collect();
            Ok(ToolOutcome::Done(json!({
                "data_verified": d.verified, "modes": ["safe", "ui", "device"],
                "states": ["default", "theatre", "fullscreen", "comments", "desc", "share", "more", "profile", "sound"],
                "player_views": {"note": "desktop long-form only: px of a 1920x1080 viewport", "platforms": d.platforms.iter().filter(|p| !p.vw.is_empty()).map(|p| (p.id.clone(), json!(p.vw))).collect::<serde_json::Map<String, Value>>()},
                "devices": devices, "platforms": platforms, "current": current(app),
                "user_file": social::user_path().display().to_string(),
            })))
        },
    },
    ToolDef {
        name: "guides.set",
        desc: "Choose what the viewer's social guide shows: platform ('off' hides it), mode (safe|ui|device), device class, the active profile of that platform's network, worst-case, keep-aspect, draw-in-render, and which sheet a device preview has open (state). Only the view changes - never the project or the export.",
        args: &[
            "platform:string:false:a guides.list id, or off",
            "mode:string:false:safe|ui|device",
            "device:string:false:a guides.list device id",
            "profile:string:false:profile id (becomes the active one for the platform's network)",
            "worst_case:boolean:false:",
            "keep_aspect:boolean:false:",
            "draw_in_render:boolean:false:also draw it in render.frame / render.contact_sheet",
            "state:string:false:device preview: default|theatre|fullscreen (desktop long form) or a sheet: comments|desc|share|more|profile|sound",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let d = social::data();
            if let Some(p) = arg_str(args, "platform") {
                if p == "off" || p.is_empty() {
                    app.settings.guide = None;
                } else if d.platform(p).is_some() {
                    app.settings.guide = Some(p.to_string());
                } else {
                    return Err(format!("unknown platform '{p}' (guides.list)"));
                }
            }
            if let Some(m) = arg_str(args, "mode") {
                if !Mode::ALL.iter().any(|x| x.id() == m) {
                    return Err("mode: safe|ui|device".into());
                }
                app.settings.guide_mode = m.to_string();
            }
            if let Some(v) = arg_str(args, "device") {
                if !d.devices.iter().any(|x| x.id == v) {
                    return Err(format!("unknown device '{v}' (guides.list)"));
                }
                app.settings.guide_device = v.to_string();
            }
            if let Some(id) = arg_str(args, "profile") {
                let p = app.settings.social_profiles.iter().find(|p| p.id == id).ok_or_else(|| format!("no profile '{id}' (profile.get)"))?;
                let net = p.net.clone();
                app.settings.social_active.insert(net, id.to_string());
            }
            if let Some(b) = arg_bool(args, "worst_case") {
                app.settings.guide_worst = b;
            }
            if let Some(b) = arg_bool(args, "keep_aspect") {
                app.settings.guides_keep_aspect = b;
            }
            if let Some(b) = arg_bool(args, "draw_in_render") {
                app.settings.guide_render = b;
            }
            if args.get("state").is_some() {
                app.preview.social.want = state_arg(args)?;
                if let Some(v) = view_arg(args) {
                    app.settings.guide_view = v.id().into();
                }
            }
            app.settings.save();
            Ok(ToolOutcome::Done(json!({"ok": true, "current": current(app)})))
        },
    },
    ToolDef {
        name: "guides.preview",
        desc: "A picture of the guide: the project's frame at time t inside the device (mode device, the default) with the platform UI and the chosen sheet open, or the frame with the safe-zone / UI-preview overlay. Uses the active profile (or `profile`). Nothing in the project or the viewer changes.",
        args: &[
            "platform:string:false:default = the current guide, else tiktok",
            "device:string:false:default = the current device",
            "mode:string:false:device (default)|safe|ui",
            "state:string:false:default|theatre|fullscreen|comments|desc|share|more|profile|sound",
            "profile:string:false:profile id to show",
            "t:number:false:default = the playhead",
            "width:integer:false:picture width in px (default 540)",
        ],
        kind: ToolKind::Read,
        run: |app, args| {
            let d = social::data();
            let mut s: Settings = app.settings.clone();
            let plat = arg_str(args, "platform").map(str::to_string).or_else(|| s.guide.clone()).unwrap_or_else(|| "tiktok".into());
            let platform = d.platform(&plat).ok_or_else(|| format!("unknown platform '{plat}' (guides.list)"))?;
            s.guide = Some(plat.clone());
            if let Some(v) = arg_str(args, "device") {
                s.guide_device = v.to_string();
            }
            s.guide_mode = arg_str(args, "mode").unwrap_or("device").to_string();
            if let Some(v) = view_arg(args) {
                s.guide_view = v.id().into();
                if arg_str(args, "device").is_none() && v != ViewMode::Default && platform.pg == "watch" {
                    s.guide_device = "desktop".into(); // the player views are a desktop thing
                }
            }
            if let Some(id) = arg_str(args, "profile") {
                if !s.social_profiles.iter().any(|p| p.id == id && p.net == platform.net) {
                    return Err(format!("no {} profile '{id}' (profile.get)", platform.net));
                }
                s.social_active.insert(platform.net.clone(), id.to_string());
            }
            let open = state_arg(args)?;
            let t = arg_f64(args, "t").unwrap_or(app.playhead);
            let width = arg_u64(args, "width").map(|w| w as u32).unwrap_or(540).clamp(160, 2400);
            let aspect = app.project.width.max(1) as f32 / app.project.height.max(1) as f32;
            let title = app.project.name.clone();
            let dur = app.project.duration();
            let frame = app.render_frame_now(t, width.min(app.project.width.max(16)));
            let g = GuideView::new(&s, &title, aspect, t, dur).ok_or("no such platform")?;
            let out = match g.mode {
                Mode::Device => raster::render_device(frame.as_deref(), &g, open, width),
                _ => match &frame {
                    Some(f) => raster::render_overlay((**f).clone(), &g),
                    None => return Err("render timed out".into()),
                },
            };
            let meta = json!({"platform": plat, "device": s.guide_device, "mode": g.mode.id(), "state": open.map_or(s.guide_view.as_str(), |x| x.id()), "t": t,
                              "width": out.width, "height": out.height, "profile": g.profile.id, "verified": platform.ver});
            Ok(ToolOutcome::Image { png_b64: base64(&crate::mcp::png_encode(&out)), meta })
        },
    },
    ToolDef {
        name: "profile.get",
        desc: "The social profiles (fake accounts the guide preview shows): all of them, or one by id. `active` names the one used per network. No credentials are ever stored.",
        args: &["id:string:false:omit = all", "network:string:false:youtube|tiktok|instagram|facebook|snapchat|x"],
        kind: ToolKind::Read,
        run: |app, args| {
            let s = &app.settings;
            if let Some(id) = arg_str(args, "id") {
                let p = s.social_profiles.iter().find(|p| p.id == id).ok_or_else(|| format!("no profile '{id}'"))?;
                return Ok(ToolOutcome::Done(profile_json(p)));
            }
            let net = arg_str(args, "network");
            let list: Vec<Value> = s.social_profiles.iter().filter(|p| net.is_none_or(|n| p.net == n)).map(profile_json).collect();
            Ok(ToolOutcome::Done(json!({"profiles": list, "active": s.social_active})))
        },
    },
    ToolDef {
        name: "profile.set",
        desc: "Create or edit a social profile. `id` edits that profile; with `network` and no id a new one starts from the sample. `fields`: name, handle, display, verified, followers, caption, tags, sound, likes, comments, shares, saves, views, link, comment_list [{name,text,likes,when}]. `avatar_path` = a picture file. `active` makes it the one used for its network; `delete` removes it.",
        args: &[
            "id:string:false:",
            "network:string:false:youtube|tiktok|instagram|facebook|snapchat|x",
            "fields:object:false:the text fields to set",
            "avatar_path:string:false:a local picture file (cropped to 96x96)",
            "active:boolean:false:",
            "delete:boolean:false:",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = match arg_str(args, "id") {
                Some(id) if app.settings.social_profiles.iter().any(|p| p.id == id) => id.to_string(),
                Some(id) => return Err(format!("no profile '{id}' (profile.get)")),
                None => {
                    let net = req(arg_str(args, "network"), "network (or an id)")?;
                    if !profile::NETS.iter().any(|n| n.0 == net) {
                        return Err("network: youtube|tiktok|instagram|facebook|snapchat|x".into());
                    }
                    let mut p = SocialProfile::sample(net);
                    p.id = app.settings.new_profile_id();
                    p.name = format!("Profile {}", app.settings.social_profiles.len() + 1);
                    let id = p.id.clone();
                    app.settings.social_profiles.push(p);
                    id
                }
            };
            if arg_bool(args, "delete") == Some(true) {
                app.settings.social_profiles.retain(|p| p.id != id);
                app.settings.social_active.retain(|_, v| *v != id);
                app.settings.save();
                return Ok(ToolOutcome::Done(json!({"ok": true, "deleted": id})));
            }
            let avatar = match arg_str(args, "avatar_path") {
                Some(p) => Some(profile::avatar_from_file(std::path::Path::new(p))?),
                None => None,
            };
            let p = app.settings.social_profiles.iter_mut().find(|p| p.id == id).expect("just found");
            if let Some(f) = args.get("fields") {
                set_profile_fields(p, f)?;
            }
            if let Some(a) = avatar {
                p.avatar = a;
            }
            let (net, out) = (p.net.clone(), profile_json(p));
            if arg_bool(args, "active") == Some(true) {
                app.settings.social_active.insert(net, id);
            }
            app.settings.save();
            Ok(ToolOutcome::Done(json!({"ok": true, "profile": out})))
        },
    },
    ToolDef {
        name: "profile.fetch",
        desc: "Fill a profile from its PUBLIC page (no login, no cookies): downloads that one page and its og:image with curl, reads the name, picture and follower text, applies them (unless apply is false) and returns exactly what was found and from where. Blocks the editor for up to ~40 s, and the Assistant guard asks the user first: it only ever runs on their say-so.",
        args: &[
            "id:string:true:the profile to fill",
            "url:string:false:public profile URL or @handle (default: the profile's link, else its handle)",
            "apply:boolean:false:default true",
        ],
        kind: ToolKind::Ui,
        run: |app, args| {
            let id = req(arg_str(args, "id"), "id")?;
            let p = app.settings.social_profiles.iter().find(|p| p.id == id).ok_or_else(|| format!("no profile '{id}' (profile.get)"))?;
            let input = arg_str(args, "url").map(str::to_string).unwrap_or_else(|| if p.link.is_empty() { p.handle.clone() } else { p.link.clone() });
            let f = profile::fetch(&p.net.clone(), &input)?;
            let report = f.report();
            let apply = arg_bool(args, "apply").unwrap_or(true);
            if apply {
                if let Some(p) = app.settings.social_profiles.iter_mut().find(|p| p.id == id) {
                    f.apply(p);
                }
                app.settings.save();
            }
            Ok(ToolOutcome::Done(json!({"ok": true, "applied": apply, "requested": f.url, "report": report, "found": {
                "title": f.title, "description": f.description, "image_url": f.image_url, "display": f.display, "followers": f.followers, "avatar": !f.avatar.is_empty()}})))
        },
    },
];
