// @ts-check
/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
module.exports = {
  tutorial: [
    'getting-started',
    'interface',
    'screens',
    'media-player',
    {
      type: 'category',
      label: 'Editing',
      collapsed: false,
      items: ['library', 'project-settings', 'screen-recording', 'viewer', 'social-guides', 'device-emulator', 'timeline', 'inspector', 'animator', 'effects', 'html-layers', 'chat-forum-threads', 'browser-layer', 'online-library', 'my-gallery', 'templates'],
    },
    {
      type: 'category',
      label: 'Finishing',
      collapsed: false,
      items: ['captions', 'color', 'audio', 'audio-visualizers', 'beat-aligner', 'music-and-discs', 'export', 'export-audio-tracks', 'faster-exports', 'handoff'],
    },
    {
      type: 'category',
      label: 'Editing with AI',
      collapsed: false,
      items: ['assistant', 'project-context', 'editing-styles', 'editing-skills', 'voice'],
    },
    {
      type: 'category',
      label: 'Reference',
      collapsed: false,
      items: ['right-click', 'shortcuts', 'advanced', 'scripting', 'report-a-bug'],
    },
    {
      type: 'category',
      label: 'Guides',
      collapsed: false,
      items: ['making-the-trailer'],
    },
  ],
};
