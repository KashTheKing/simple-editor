---
name: blocks-from-keys
description: Convert the timeline's hand-made keyframes into animation blocks and its per-cut transitions into transition spans, the motion unchanged.
args: "[optional: a track, a time range, include hard cuts]"
tags: animator, keyframes, blocks, transitions, spans, simplify, convert
---
Turn the keyframes made by hand into animation blocks (the bars in the Animator panel, each with controls) and group the transitions on the cuts into transition spans, so the edit is simpler to change. The picture must look exactly the same afterwards. Do the safe thing without asking: this request alone is enough to go ahead.

1. `animate.audit {}` (one small read: never page through `timeline.list` for this). It lists the clips with hand keys (`hand_runs`), the blocks already there, and `transitions`: cuts with and without a transition, their kinds and lengths, and `suggested_spans`. Use $ARGS to narrow the scope (a track name, `from`/`to` seconds) in every call below.
2. Keys to blocks: `keyblocks.convert_all {all: true}` (or the scope from $ARGS). It turns every run of hand keys into a named block when one reproduces it exactly (Fade In, Fade Out, Slide, Pop, Zoom Punch, Ken Burns, Sound Swell, ...: with strength, ease and origin / target set) and into a "Custom" block of the same keys otherwise. It skips keys a block already owns and checks every property before and after; it never changes the motion, so there is no need to preview with `dry_run` first.
3. Transitions to spans: `animate.spans_from_transitions {}`. It makes one span per run of cuts that already share a transition and unifies their lengths within the run. Hard cuts stay hard: only add `include_hard_cuts: true` when the user asked for transitions on every cut. If the audit shows no transitions at all, skip this step and say so.
4. Check: `animate.audit {}` again (`clips_with_hand_keys` should be 0 or only the skipped ones; `transitions.in_spans` = the cuts grouped) and `render.contact_sheet {from, to, count: 6}` over a stretch with blocks and spans.

This whole request is one undo step. Do not call `edit.begin` / `edit.end` inside the Assistant.

Request: $ARGS

Report back: how many clips and blocks were converted (named by kind, how many Custom), anything skipped and why, how many spans were made over how many cuts and at what length, how many hard cuts were left hard, and that one Undo restores everything.
