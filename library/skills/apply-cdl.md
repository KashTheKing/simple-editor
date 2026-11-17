---
name: apply-cdl
description: Apply an ASC CDL grade (slope, offset, power, saturation) from Resolve, Avid or a colourist to clips.
args: "<slope r g b, offset r g b, power r g b, sat s>  or paste the .cdl / .cc XML"
tags: cdl, asc, grade, resolve, avid, color, converted
credit: converted from the ASC CDL import step of the open DaVinci Resolve MCP server (samuelgursky/davinci-resolve-mcp, MIT) through /convert-from-editor; CDL maths is the public ASC formula
---
A CDL is ten numbers that describe a primary grade: per channel slope (gain), offset (lift) and power (gamma), plus saturation. Colourists, on-set tools, Resolve and Avid exchange them. The Primaries effect computes `(in x gain + lift) ^ (1 / gamma)`, which is the CDL formula, so slope, offset and power carry over exactly (values clamped to 0-1, as in most grading tools); saturation goes on a second effect and is close but not identical (CDL uses Rec. 709 luma weights).

1. Parse $ARGS. Accept `<Slope>`, `<Offset>`, `<Power>`, `<Saturation>` from pasted XML (.cdl / .cc / .ccc; with several ColorCorrection entries ask which, or match them to clips by id or name), or plain numbers. Missing values default to slope 1, offset 0, power 1, sat 1. If nothing parses, ask for the numbers.
2. Call `editor.context` and `timeline.list`. Targets: the selected video clips; with none, ask. Look first with `render.frame {t}` at one target.
3. Map the numbers (the CDL formula is out = (in x slope + offset) ^ power, then saturation):
   - `gain_r`, `gain_g`, `gain_b` = slope r, g, b.
   - `lift_r`, `lift_g`, `lift_b` = offset r, g, b (added after the gain, exactly like the CDL offset; usually within +-0.1).
   - `gamma_r`, `gamma_g`, `gamma_b` = 1 / power r, g, b (the effect raises to 1 / gamma, so power 1.2 becomes gamma 0.833).
   - Saturation: when sat is not 1, add `clip.add_effect {clip_id, kind: "Hue / Saturation", params}` with the saturation parameter set to sat (check the parameter's name and range in `timeline.list` after adding it; scale if the effect uses a percent).
4. Apply in one group: `edit.begin {label: "Apply CDL"}`; for each target `color.primaries {clip_id, gain_r, gain_g, gain_b, lift_r, lift_g, lift_b, gamma_r, gamma_g, gamma_b}`; then the saturation effect; `edit.end`. Existing grade effects stay; the Primaries effect is created or updated.
5. Verify: `render.frame {t}` on each target and `frame.stats {t}` before and after. If a reference image of the intended grade exists, compare against it and say how close it is. Clips shot in log need their conversion LUT first (`clip.add_lut`), because a CDL is defined on the working image.

CDL given: $ARGS

Report back: the values applied per channel, the clips graded, that slope / offset / power are exact and saturation is close, and that one Undo removes it.
