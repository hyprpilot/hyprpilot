import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Hyprpilot',
  description: 'A config-driven launcher that resolves a profile and execs your coding agent’s native CLI.',
  cleanUrls: true,
  lastUpdated: true,
  head: [
    ['link', { rel: 'icon', type: 'image/png', href: '/icon.png' }],
    ['meta', { name: 'theme-color', content: '#e5c07b' }]
  ],
  themeConfig: {
    logo: '/icon.png',
    siteTitle: 'hyprpilot',
    nav: [
      { text: 'Guide', link: '/guide/installation' },
      { text: 'Configuration', link: '/configuration/' },
      { text: 'Reference', link: '/reference/cli' },
      { text: 'Repository', link: '/repository/contributions' }
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/hyprpilot/hyprpilot' }
    ],
    search: {
      provider: 'local'
    },
    editLink: {
      pattern: 'https://github.com/hyprpilot/hyprpilot/edit/main/docs/:path',
      text: 'Edit this page on GitHub'
    },
    footer: {
      message: 'MIT licensed.',
      copyright: 'Copyright © 2026 Cenk Kılıç'
    },
    outline: {
      level: [2, 3]
    },
    // Single shared sidebar across every section, so the reader sees
    // the whole TOC at all times and prev / next walks the full
    // tree linearly. Links are absolute (`/guide/...`) so VitePress
    // can match the current page and resolve prev / next correctly.
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Installation', link: '/guide/installation' },
          { text: 'Integration', link: '/guide/integration' }
        ]
      },
      {
        text: 'Configuration',
        items: [
          { text: 'Overview', link: '/configuration/' },
          { text: 'Profiles', link: '/configuration/profiles' },
          { text: 'Agents', link: '/configuration/agents' },
          { text: 'MCP & skills', link: '/configuration/mcp-and-skills' },
          { text: 'Patches & overlays', link: '/configuration/patches' }
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'CLI', link: '/reference/cli' },
          { text: 'MCP server', link: '/reference/mcp-server' }
        ]
      },
      {
        text: 'Repository',
        items: [
          { text: 'Contributions', link: '/repository/contributions' },
          { text: 'Release', link: '/repository/release' },
          { text: 'Development', link: '/repository/development' }
        ]
      }
    ]
  },
  markdown: {
    theme: {
      light: 'one-light',
      dark: 'one-dark-pro'
    },
    lineNumbers: false
  },
  sitemap: {
    hostname: 'https://hyprpilot.kilic.dev'
  }
})
