import { defineConfig } from 'vitepress'
import llmstxt from 'vitepress-plugin-llms'
import { generateSidebar } from 'vitepress-sidebar'

export default defineConfig({
  title: 'Hyprpilot',
  description: 'A config-driven, fire-and-exec launcher for terminal coding agents.',
  cleanUrls: true,
  lastUpdated: true,
  // Historical planning notes live in docs/plans/ for archaeology only —
  // they never become site pages.
  srcExclude: ['plans/**'],
  head: [
    ['link', { rel: 'icon', type: 'image/png', href: '/icon.png' }],
    ['meta', { name: 'theme-color', content: '#e5c07b' }]
  ],
  themeConfig: {
    logo: '/icon.png',
    siteTitle: 'hyprpilot',
    nav: [
      { text: 'Config', link: '/config/' },
      { text: 'Runtime', link: '/runtime/' },
      { text: 'Repository', link: '/repository/foreword' }
    ],
    socialLinks: [{ icon: 'github', link: 'https://github.com/hyprpilot/hyprpilot' }],
    search: {
      provider: 'local'
    },
    editLink: {
      pattern: 'https://github.com/hyprpilot/hyprpilot/edit/main/docs/:path',
      text: 'Edit this page on GitHub'
    },
    footer: {
      message: `
<img src="https://main.s3.kilic.dev/html/icon.png" style="max-height: 16px;" />
<a href="https://kilic.dev" target="_blank">kilic.dev</a>
<br/>
<small>MIT licensed. Made with <a href="https://vitepress.dev/" target="_blank">Vitepress</a>.</small>
`
    },
    outline: {
      level: [2, 3]
    },
    // Sidebar is generated from the directory tree: page order comes from
    // each page's frontmatter `order:`, section order from
    // `manualSortFileNameByPriority`.
    sidebar: generateSidebar({
      documentRootPath: '/',
      useTitleFromFrontmatter: true,
      sortMenusByFrontmatterOrder: true,
      capitalizeFirst: true,
      collapsed: true,
      includeFolderIndexFile: true,
      excludePattern: ['plans/**', 'node_modules/**', 'dist/**'],
      manualSortFileNameByPriority: ['config', 'runtime', 'repository']
    }) as any[]
  },
  markdown: {
    theme: {
      light: 'one-light',
      dark: 'one-dark-pro'
    },
    lineNumbers: true
  },
  vite: {
    clearScreen: false,
    plugins: [
      // llms.txt / llms-full.txt + per-page markdown for LLM consumption.
      llmstxt({
        domain: 'https://hyprpilot.kilic.dev'
      })
    ]
  },
  sitemap: {
    hostname: 'https://hyprpilot.kilic.dev'
  }
})
