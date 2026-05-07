import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'Hyprpilot',
  description: 'An overlay daemon that runs coding agents at the edge of your screen.',
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
      { text: 'Features', link: '/features/' },
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
          { text: 'Integration', link: '/guide/integration' },
          { text: 'Waybar', link: '/guide/waybar' }
        ]
      },
      {
        text: 'Configuration',
        items: [
          { text: 'Overview', link: '/configuration/' },
          { text: 'Profiles', link: '/configuration/profiles' },
          { text: 'Agents', link: '/configuration/agents' },
          { text: 'Window', link: '/configuration/window' },
          { text: 'Theme', link: '/configuration/theme' },
          { text: 'Remote bridge', link: '/configuration/remote' }
        ]
      },
      {
        text: 'Features',
        items: [
          { text: 'Overview', link: '/features/' },
          { text: 'Command palette', link: '/features/command-palette' },
          { text: 'Chat & tools', link: '/features/chat-and-tools' },
          { text: 'Composer', link: '/features/composer' },
          { text: 'Daemon & CLI', link: '/features/cli' },
          { text: 'Remote bridge', link: '/features/remote' }
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
