// @ts-check
// Docs-only Docusaurus site for Simple Editor, published to GitHub Pages by .github/workflows/docs.yml.
// Screenshots come from scripts/docs-shots.ps1, the shortcuts page from `simple-editor --dump-hotkeys`.

/** @type {import('@docusaurus/types').Config} */
const config = {
  title: 'Simple Editor',
  tagline: 'A fast, simple video editor for Windows',
  favicon: 'img/favicon.ico',
  url: 'https://kashtheking.github.io',
  baseUrl: '/simple-editor/',
  organizationName: 'KashTheKing',
  projectName: 'simple-editor',
  trailingSlash: false,
  onBrokenLinks: 'throw',
  onBrokenAnchors: 'throw',
  markdown: {
    hooks: { onBrokenMarkdownLinks: 'throw', onBrokenMarkdownImages: 'throw' },
  },
  i18n: { defaultLocale: 'en', locales: ['en'] },
  presets: [
    [
      'classic',
      /** @type {import('@docusaurus/preset-classic').Options} */
      ({
        docs: {
          routeBasePath: '/',
          sidebarPath: './sidebars.js',
          editUrl: 'https://github.com/KashTheKing/simple-editor/edit/main/website/',
        },
        blog: false,
        theme: { customCss: './src/css/custom.css' },
      }),
    ],
  ],
  themeConfig:
    /** @type {import('@docusaurus/preset-classic').ThemeConfig} */
    ({
      colorMode: { defaultMode: 'dark', respectPrefersColorScheme: true },
      navbar: {
        title: 'Simple Editor',
        logo: { alt: 'Simple Editor', src: 'img/logo.png' },
        items: [
          { to: '/', label: 'Tutorial', position: 'left', activeBaseRegex: '^/simple-editor/?$' },
          { to: '/shortcuts', label: 'Shortcuts', position: 'left' },
          {
            href: 'https://github.com/KashTheKing/simple-editor/releases/latest',
            label: 'Download',
            position: 'right',
          },
          { href: 'https://github.com/KashTheKing/simple-editor', label: 'GitHub', position: 'right' },
        ],
      },
      footer: {
        style: 'dark',
        links: [
          {
            title: 'Simple Editor',
            items: [
              { label: 'Download', href: 'https://github.com/KashTheKing/simple-editor/releases/latest' },
              { label: 'Source on GitHub', href: 'https://github.com/KashTheKing/simple-editor' },
            ],
          },
          {
            title: 'Community',
            items: [
              { label: 'Discord', href: 'https://discord.gg/6taNJVs5FT' },
              { label: 'Report a bug', href: 'https://github.com/KashTheKing/simple-editor/issues' },
            ],
          },
        ],
        copyright: `Simple Editor. Screenshots are generated from the app by scripts/docs-shots.ps1.`,
      },
    }),
};

module.exports = config;
