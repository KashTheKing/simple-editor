use super::*;

// ---- ws:audio-analysis ----
/// A `peaks_of` closure over `waveforms`, keyed by ASSET id (stream 0) - the shape
/// `engine::analysis::normalize`/`match_loudness`/`duck` take. Shared by `tools_audio.rs` and
/// `audio_actions.rs`. ponytail: a clip picking a non-zero `audio_stream` reads stream 0 here; correct
/// for the overwhelming single-stream-asset case.
pub(super) fn asset_peaks(
    waveforms: &mut crate::media::waveform::WaveformCache,
) -> impl FnMut(&Project, Id) -> Option<Arc<crate::media::waveform::Peaks>> + '_ {
    move |project, asset_id| {
        let path = project.asset(asset_id)?.path.clone();
        waveforms.get(&path, 0)
    }
}

pub(super) fn add_mask(project: &mut Project, clip: Id, shape: MaskShape) -> bool {
    let Some(c) = project.clip_mut(clip).filter(|c| c.is_visual()) else { return false };
    match c.effects.iter_mut().rev().find(|e| e.enabled) {
        Some(e) if e.mask.is_none() => {
            e.mask = Some(Mask::new(shape));
            true
        }
        Some(_) => false,
        None if c.mask.is_none() => {
            c.mask = Some(Mask::new(shape));
            true
        }
        None => false,
    }
}

/// Preview render size for a pane of `canvas` px at `quality` percent (25..100). Zero stays zero
/// (nothing to render), and the aspect ratio is kept.

pub(super) fn arg_str<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)?.as_str()
}
pub(super) fn arg_f64(args: &Value, k: &str) -> Option<f64> {
    args.get(k)?.as_f64()
}
pub(super) fn arg_u64(args: &Value, k: &str) -> Option<u64> {
    args.get(k)?.as_u64()
}
pub(super) fn arg_bool(args: &Value, k: &str) -> Option<bool> {
    args.get(k)?.as_bool()
}
pub(super) fn arg_ids(args: &Value, k: &str) -> Option<Vec<Id>> {
    Some(args.get(k)?.as_array()?.iter().filter_map(|v| v.as_u64()).collect())
}
pub(super) fn req<T>(o: Option<T>, k: &str) -> Result<T, String> {
    o.ok_or_else(|| format!("missing or invalid argument '{k}'"))
}

/// "Linear" | "EaseIn" | … | "cubic-bezier(x1,y1,x2,y2)"
pub(super) fn parse_ease(s: &str) -> Option<crate::model::Ease> {
    use crate::model::Ease;
    match s {
        "Linear" => return Some(Ease::Linear),
        "EaseIn" => return Some(Ease::EaseIn),
        "EaseOut" => return Some(Ease::EaseOut),
        "EaseInOut" => return Some(Ease::EaseInOut),
        "Hold" => return Some(Ease::Hold),
        _ => {}
    }
    let inner = s.strip_prefix("cubic-bezier(")?.strip_suffix(')')?;
    let v: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    match v[..] {
        [x1, y1, x2, y2] => Some(Ease::Bezier { x1, y1, x2, y2 }),
        _ => None,
    }
}

/// The animated property `name` of a clip: inspector labels or "<Effect name>: <Param name>".
pub(super) fn anim_of<'a>(clip: &'a mut Clip, name: &str) -> Option<&'a mut crate::model::Animated> {
    match name {
        "Position X" => return Some(&mut clip.x),
        "Position Y" => return Some(&mut clip.y),
        "Scale" => return Some(&mut clip.scale),
        "Rotation" => return Some(&mut clip.rotation),
        "Opacity" => return Some(&mut clip.opacity),
        "Volume" => return Some(&mut clip.volume),
        "Pan" => return Some(&mut clip.pan),
        _ => {}
    }
    let (effect, param) = name.split_once(':')?;
    let (effect, param) = (effect.trim(), param.trim());
    let e = clip.effects.iter_mut().find(|e| e.kind.name().eq_ignore_ascii_case(effect))?;
    let i = e.kind.params().iter().position(|p| p.name.eq_ignore_ascii_case(param))?;
    e.params.get_mut(i)
}

pub(super) fn mask_shape(s: &str) -> Result<MaskShape, String> {
    MaskShape::ALL
        .into_iter()
        .find(|m| m.name().eq_ignore_ascii_case(s) || format!("{m:?}").eq_ignore_ascii_case(s))
        .ok_or_else(|| format!("unknown mask shape '{s}' (Rect | Ellipse | Polygon | Path)"))
}

/// The mask slot of a clip, or of one of its effects (`effect` = index in the effect stack).
pub(super) fn mask_slot(project: &mut Project, clip: Id, effect: Option<usize>) -> Result<&mut Option<Mask>, String> {
    let c = project.clip_mut(clip).ok_or("no such clip")?;
    if !c.is_visual() {
        return Err("that clip is audio - a mask shapes pixels, and audio has none".into());
    }
    match effect {
        None => Ok(&mut c.mask),
        Some(i) => Ok(&mut c.effects.get_mut(i).ok_or("no effect at that index")?.mask),
    }
}

pub(super) fn apply_mask_fields(mask: &mut Mask, fields: &Value) -> Result<(), String> {
    let obj = fields.as_object().ok_or("'fields' must be an object")?;
    for (k, v) in obj {
        match k.as_str() {
            "shape" => mask.shape = mask_shape(v.as_str().ok_or("shape: string")?)?,
            "invert" => mask.invert = v.as_bool().ok_or("invert: bool")?,
            "enabled" => mask.enabled = v.as_bool().ok_or("enabled: bool")?,
            "points" => {
                let a = v.as_array().ok_or("points: [[x, y], …]")?;
                mask.points = a
                    .iter()
                    .filter_map(|p| {
                        let p = p.as_array()?;
                        Some((p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32))
                    })
                    .collect();
            }
            "cx" | "cy" | "rx" | "ry" | "rotation" | "feather" | "expand" | "opacity" => {
                let val = v.as_f64().ok_or_else(|| format!("{k}: number"))?;
                let a = match k.as_str() {
                    "cx" => &mut mask.cx,
                    "cy" => &mut mask.cy,
                    "rx" => &mut mask.rx,
                    "ry" => &mut mask.ry,
                    "rotation" => &mut mask.rotation,
                    "feather" => &mut mask.feather,
                    "expand" => &mut mask.expand,
                    _ => &mut mask.opacity,
                };
                a.keys.clear();
                a.value = val;
            }
            _ => return Err(format!("unknown mask field '{k}'")),
        }
    }
    Ok(())
}

/// "Blur" / "Color" / "Blend" / "Combine" / "Merge" / "Matte" / "Mask" / "Text" / "Input" → a node kind.
pub(super) fn node_kind(s: &str) -> Result<NodeKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "input" => return Ok(NodeKind::Input),
        "output" => return Err("every graph already has exactly one Output".into()),
        "blend" => return Ok(NodeKind::Blend { mode: BlendMode::Normal, opacity: crate::model::Animated::new(1.0) }),
        "combine" => {
            return Ok(NodeKind::Combine { mode: BlendMode::Normal, factor: crate::model::Animated::new(0.5) })
        }
        "merge" => return Ok(NodeKind::Merge),
        "matte" => return Ok(NodeKind::Matte { invert: false, use_alpha: false }),
        "color" => return Ok(NodeKind::Color([0, 0, 0, 255])),
        "mask" => return Ok(NodeKind::Mask(Mask::new(MaskShape::Ellipse))),
        "text" | "string" => {
            return Ok(NodeKind::String(crate::model::TextStyle { text: "{time}".into(), ..Default::default() }))
        }
        _ => {}
    }
    let kind = EffectKind::ALL
        .into_iter()
        .find(|k| k.name().eq_ignore_ascii_case(s) || format!("{k:?}").eq_ignore_ascii_case(s))
        .ok_or_else(|| {
            format!(
                "unknown node kind '{s}' (an effect name, Blend, Combine, Merge, Matte, Mask, Color, Text or Input)"
            )
        })?;
    Ok(NodeKind::Effect(Effect::new(kind)))
}

pub(super) fn color_arg(v: &Value) -> Option<[u8; 4]> {
    let a = v.as_array()?;
    let mut c = [255u8; 4];
    for (i, x) in a.iter().take(4).enumerate() {
        c[i] = x.as_u64()? as u8;
    }
    (a.len() >= 3).then_some(c)
}

/// Apply `clip.set` fields to a clip (everything except `speed`/`reverse`, which the caller routes
/// through `Project::set_speed` so neighbour collisions are respected).
pub(super) fn apply_clip_fields(clip: &mut Clip, fields: &Value) -> Result<(), String> {
    let obj = fields.as_object().ok_or("'fields' must be an object")?;
    for (k, v) in obj {
        match k.as_str() {
            "speed" | "reverse" => {} // handled by the caller
            "name" => clip.name = v.as_str().ok_or("name: string")?.to_string(),
            "enabled" => clip.enabled = v.as_bool().ok_or("enabled: bool")?,
            "label" => clip.label = v.as_u64().filter(|&l| l <= 8).ok_or("label: 0..8")? as u8,
            "freeze" => clip.freeze = if v.is_null() { None } else { Some(v.as_f64().ok_or("freeze: number|null")?) },
            "blend" => {
                let name = v.as_str().ok_or("blend: string")?;
                clip.blend = BlendMode::ALL
                    .into_iter()
                    .find(|b| b.name().eq_ignore_ascii_case(name) || format!("{b:?}").eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("unknown blend mode '{name}'"))?;
            }
            "fade_in" => clip.fade_in = v.as_f64().ok_or("fade_in: number")?.max(0.0),
            "fade_out" => clip.fade_out = v.as_f64().ok_or("fade_out: number")?.max(0.0),
            "x" | "y" | "scale" | "rotation" | "opacity" | "volume" | "pan" => {
                let val = v.as_f64().ok_or_else(|| format!("{k}: number"))?;
                let a = match k.as_str() {
                    "x" => &mut clip.x,
                    "y" => &mut clip.y,
                    "scale" => &mut clip.scale,
                    "rotation" => &mut clip.rotation,
                    "opacity" => &mut clip.opacity,
                    "volume" => &mut clip.volume,
                    _ => &mut clip.pan,
                };
                a.keys.clear(); // "constant value": drop any animation
                a.value = val;
            }
            // text style fields
            _ => {
                let Some(t) = clip.text.as_mut() else {
                    return Err(format!("unknown clip field '{k}' (text fields need a text clip)"));
                };
                match k.as_str() {
                    "text" => t.text = v.as_str().ok_or("text: string")?.to_string(),
                    "font" => t.font = v.as_str().ok_or("font: string")?.to_string(),
                    // ws:text-titles: size/outline_width/letter_spacing are now Animated - `clip.set`
                    // is a discrete "set the value" op, so it clears any keys, same as x/y/scale above.
                    "size" => {
                        t.size.keys.clear();
                        t.size.value = v.as_f64().ok_or("size: number")?;
                    }
                    "bold" => t.bold = v.as_bool().ok_or("bold: bool")?,
                    "italic" => t.italic = v.as_bool().ok_or("italic: bool")?,
                    "color" => t.color = color_arg(v).ok_or("color: [r,g,b,a]")?,
                    "outline_width" => {
                        t.outline_width.keys.clear();
                        t.outline_width.value = v.as_f64().ok_or("outline_width: number")?;
                    }
                    "outline_color" => t.outline_color = color_arg(v).ok_or("outline_color: [r,g,b,a]")?,
                    "shadow" => t.shadow = v.as_bool().ok_or("shadow: bool")?,
                    "shadow_color" => t.shadow_color = color_arg(v).ok_or("shadow_color: [r,g,b,a]")?,
                    "shadow_x" => t.shadow_x = v.as_f64().ok_or("shadow_x: number")? as f32,
                    "shadow_y" => t.shadow_y = v.as_f64().ok_or("shadow_y: number")? as f32,
                    "shadow_blur" => t.shadow_blur = v.as_f64().ok_or("shadow_blur: number")? as f32,
                    "align" => t.align = v.as_u64().filter(|&a| a <= 2).ok_or("align: 0..2")? as u8,
                    "line_spacing" => t.line_spacing = v.as_f64().ok_or("line_spacing: number")? as f32,
                    "letter_spacing" => {
                        t.letter_spacing.keys.clear();
                        t.letter_spacing.value = v.as_f64().ok_or("letter_spacing: number")?;
                    }
                    "box_color" => t.box_color = color_arg(v).ok_or("box_color: [r,g,b,a]")?,
                    "box_padding" => t.box_padding = v.as_f64().ok_or("box_padding: number")? as f32,
                    _ => return Err(format!("unknown clip field '{k}'")),
                }
            }
        }
    }
    Ok(())
}
