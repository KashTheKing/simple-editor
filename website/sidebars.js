// @ts-check
/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
module.exports = {
  tutorial: [
    'getting-started',
    'interface',
    {
      type: 'category',
      label: 'Editing',
      collapsed: false,
      items: ['library', 'viewer', 'timeline', 'inspector', 'effects', 'html-layers', 'online-library'],
    },
    {
      type: 'category',
      label: 'Finishing',
      collapsed: false,
      items: ['captions', 'color', 'audio', 'export', 'export-audio-tracks', 'faster-exports', 'handoff'],
    },
    {
      type: 'category',
      label: 'Editing with AI',
      collapsed: false,
      items: ['assistant', 'voice'],
    },
    {
      type: 'category',
      label: 'Reference',
      collapsed: false,
      items: ['right-click', 'shortcuts', 'advanced', 'scripting', 'report-a-bug'],
    },
  ],
};
