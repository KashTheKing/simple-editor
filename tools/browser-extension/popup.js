// Snapshot the active tab with the browser's own "save as single file" (MHTML) and put it in Downloads
// as SimpleEditor-<site>-<time>.mhtml. Simple Editor (Add > Web Page Snapshot > From my browser) watches
// Downloads for exactly that name and imports the page. No data leaves the computer.
(async () => {
  const msg = document.getElementById("msg");
  const say = (text, cls) => { msg.textContent = text; msg.className = cls || ""; };
  try {
    const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
    if (!tab || !/^https?:/i.test(tab.url || "")) {
      say("Open a web page (http or https) and try again.", "bad");
      return;
    }
    const blob = await new Promise((resolve, reject) =>
      chrome.pageCapture.saveAsMHTML({ tabId: tab.id }, (b) =>
        chrome.runtime.lastError ? reject(new Error(chrome.runtime.lastError.message)) : resolve(b)));
    const host = new URL(tab.url).hostname.replace(/[^a-z0-9.-]/gi, "-");
    const name = `SimpleEditor-${host}-${Date.now()}.mhtml`;
    await chrome.downloads.download({ url: URL.createObjectURL(blob), filename: name, saveAs: false });
    say(`Saved ${name} in Downloads. Simple Editor will import it.`, "ok");
    setTimeout(() => window.close(), 1800);
  } catch (e) {
    say("Could not snapshot this page: " + e.message, "bad");
  }
})();
