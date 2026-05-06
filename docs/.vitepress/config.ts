import { defineConfig } from 'vitepress'
import { withSidebar } from 'vitepress-sidebar'

const vitePressOptions = defineConfig({
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
    }
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

const sidebarOptions = {
  documentRootPath: '/',
  scanStartPath: '.',
  resolvePath: '/',
  collapsed: false,
  capitalizeFirst: true,
  useTitleFromFrontmatter: true,
  useFolderTitleFromIndexFile: true,
  sortMenusByFrontmatterOrder: true,
  manualSortFileNameByPriority: ['index.md'],
  excludeFolders: ['screenshots', 'public', '.vitepress', 'node_modules', 'dist'],
  rootGroupText: 'Hyprpilot'
}

export default defineConfig(withSidebar(vitePressOptions, sidebarOptions))
