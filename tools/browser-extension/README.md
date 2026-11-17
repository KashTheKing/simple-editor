# Snapshot this page for Simple Editor

A tiny browser extension for Chromium-based browsers (Chrome, Edge, Brave, Vivaldi, Opera, Arc and others). One click saves the page you are
looking at, **exactly as it is on your screen** (signed in, scrolled, menus open), as one `.mhtml` file in
your Downloads folder. Simple Editor turns it into an HTML layer: real HTML and CSS you can edit, zoom,
scroll and animate. No scripts are kept; nothing leaves your computer.

## Install (once, 30 seconds)

1. Open your browser's extensions page (`chrome://extensions` in most; Edge: `edge://extensions`, Opera:
   `opera://extensions`, Vivaldi: `vivaldi://extensions`, Brave: `brave://extensions`). Simple Editor's
   **Web Page Snapshot ▸ From my browser** names the right one for the browser picked in
   **Settings ▸ General ▸ Web browser** (the Windows default unless you change it).
2. Turn on **Developer mode** (top right).
3. Click **Load unpacked** and choose this folder (`tools\browser-extension`).
4. Pin the extension (puzzle icon > pin) so its button is always there.

## Use

1. In Simple Editor: **Add > Web Page Snapshot…**, open the **From my browser** tab (it waits for a page).
2. In your browser, go to the page you want (any site) and click the extension's button
   (or press **Alt+Shift+S**).
3. The page appears in Simple Editor as a new clip. Simple Editor watches your browser's download folder
   (its own setting if you changed it, else the Windows Downloads folder).

## Firefox

Firefox has no way for an extension to save a page as one file (`.mhtml`), so this extension does not work
there (loading it as a temporary add-on from `about:debugging#/runtime/this-firefox` only shows a message
saying so). In Firefox press **Ctrl+S**, save as **Web Page, complete** (an `.html` file plus a folder), and
use **Import a saved page…** in the Web Page Snapshot window.

No extension? Press **Ctrl+S** in the browser and save as **Webpage, Single File** (`.mhtml`; Edge has it,
in other Chromium browsers it may be behind `chrome://flags/#save-page-as-mhtml`), then drop the file onto the
timeline or use **Add > Import HTML File…**. "Webpage, Complete" (an `.html` file plus a folder) also
works the same way.
