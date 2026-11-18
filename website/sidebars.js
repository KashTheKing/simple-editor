// @ts-check
/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
module.exports = {
  tutorial: [
    'getting-started',
    'interface',
    'screens',
    {
      type: 'category',
      label: 'Editing',
      collapsed: false,
      items: ['library', 'viewer', 'social-guides', 'timeline', 'inspector', 'effects', 'html-layers', 'chat-forum-threads', 'browser-layer', 'online-library', 'my-gallery'],
    },
    {
      type: 'category',
      label: 'Finishing',
      collapsed: false,
      items: ['captions', 'color', 'audio', 'audio-visualizers', 'beat-aligner', 'export', 'export-audio-tracks', 'faster-exports', 'handoff'],
    },
    {
      type: 'category',
      label: 'Editing with AI',
      collapsed: false,
      items: ['assistant', 'editing-skills', 'voice'],
    },
    {
      type: 'category',
      label: 'Reference',
      collapsed: false,
      items: ['right-click', 'shortcuts', 'advanced', 'scripting', 'report-a-bug'],
    },
  ],
};
