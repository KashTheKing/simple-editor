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
      items: ['library', 'viewer', 'timeline', 'inspector', 'effects'],
    },
    {
      type: 'category',
      label: 'Finishing',
      collapsed: false,
      items: ['captions', 'color', 'audio', 'export'],
    },
    {
      type: 'category',
      label: 'Reference',
      collapsed: false,
      items: ['right-click', 'shortcuts', 'advanced', 'scripting'],
    },
  ],
};
