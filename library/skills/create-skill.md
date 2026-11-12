---
name: create-skill
description: Write a new assistant skill from your description and save it.
args: "<what the skill should do>"
---
Turn the user's description into a complete, working skill file and save it so they can run it as `/name`.

1. Read $ARGS. If it is empty or too vague to act on (no clear task), ask one question: "What should it do, in a sentence or two?" Do not guess a task.
2. Call `skills.list` to see existing names and avoid a clash. If a similar skill exists, tell the user and ask whether to make a new one or change it (`skills.get {name}` shows it). Only use tools that really exist in this app (the ones you can call).
3. Choose a `name`: lowercase letters, digits and `-` only, 1-3 words (`vlog-intro`), not an existing one.
4. Write the file in this format:
   - `description`: one line, under 90 characters, plain words, says what it does.
   - `args`: a short hint in square brackets or angle brackets if the skill takes input (for example `"[style]"`), otherwise leave it out.
   - `body`: Markdown. First sentence states the goal. Then 25-60 lines of numbered steps: what to inspect first (`editor.context`, `timeline.list`, `media.list`, check a transcript with `transcript.get`), which tools to call in order with their key arguments, what to verify afterwards (`render.frame`, `render.contact_sheet`), and where to stop and ask (nothing selected, several possible targets, slow or destructive actions, exports). Say what `$ARGS` means and what to do when it is empty. Wrap edits in `edit.begin {label}` ... `edit.end` so one Undo reverts the skill. Never delete media files; never export or overwrite without asking. End with a line telling the assistant what to report (what changed, where, how to undo).
5. Use only real tools. If the description needs something the app cannot do (for example stock footage search), write the step as "tell the user" or use the user's library media, never invent a tool.
6. Show the user the draft briefly (name, description, the step headings), then save: `skills.save {name, description, args, body}`. Saving a new file is what they asked for, so no extra confirmation is needed unless it replaces an existing skill (then ask first).
7. Verify with `skills.get {name}` that it reads back correctly.

What the user wants the skill to do: $ARGS

Report back: the new command (`/name`), its one-line description, and how to change it (open it in the Skills tab, or run `/create-skill` again for a different version). Saving a skill changes no project, so there is nothing to undo; `skills.delete {name}` removes it.
