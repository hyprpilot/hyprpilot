import tailwindcss from '@tailwindcss/vite'
import vue from '@vitejs/plugin-vue'
import path from 'node:path'
import { defineConfig } from 'vite'

const host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  resolve: {
    alias: {
      '@ipc': path.resolve(__dirname, './src/ipc'),
      '@lib': path.resolve(__dirname, './src/lib'),
      '@components': path.resolve(__dirname, './src/components'),
      '@composables': path.resolve(__dirname, './src/composables'),
      '@views': path.resolve(__dirname, './src/views'),
      '@interfaces': path.resolve(__dirname, './src/interfaces'),
      '@constants': path.resolve(__dirname, './src/constants'),
      '@adapters': path.resolve(__dirname, './src/adapters'),
      '@assets': path.resolve(__dirname, './src/assets')
    }
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
        protocol: 'ws',
        host,
        port: 1421
      }
      : undefined,
    watch: {
      ignored: ['**/src-tauri/**']
    }
  },
  build: {
    target: process.env.TAURI_ENV_PLATFORM === 'windows' ? 'chrome105' : 'safari13',
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    // The default 500 KB warning fires on shiki's highlighter core
    // (still ~800 KB after its own per-language grammar split). Bumping
    // to 1 MB silences the noise without hiding genuine regressions.
    chunkSizeWarningLimit: 1000,
    // Manual chunk groups via Vite 8 / Rolldown's `codeSplitting.groups`.
    // The default behaviour bundled every static dep into one entry
    // chunk (~2 MB) and only kept shiki's grammar files async. With
    // these groups Rolldown produces a stable vendor floor (vue +
    // tanstack), isolates the heavy per-route deps (markdown / shiki /
    // qr / xterm), and keeps the app entry small. Order matters —
    // `priority` resolves overlaps; higher priority wins.
    rolldownOptions: {
      output: {
        codeSplitting: {
          minSize: 20_000,
          groups: [
            // Shiki: deliberately NOT a manual group. Shiki's
            // `dist/langs/<lang>.mjs` + `dist/themes/<theme>.mjs`
            // entries are dynamic-imported by Shiki's core at
            // highlight time, and an over-broad `shiki|@shikijs`
            // match pulls every grammar (~10 MB worth!) into one
            // sync chunk, defeating the async loader. Letting
            // Rolldown's default chunker handle Shiki produces
            // per-grammar async chunks + a smaller core landing in
            // `vendor`. If we ever want to pre-bundle the core
            // specifically, the regex needs a negative lookahead
            // for `dist/langs/` and `dist/themes/`.

            // Markdown pipeline: markdown-it + plugins + dompurify +
            // their utility deps. Only `MarkdownBody.vue` reaches it.
            {
              name: 'markdown',
              test: /[\\/]node_modules[\\/](markdown-it|markdown-it-task-lists|dompurify|linkify-it|entities|mdurl|uc\.micro|punycode\.js)[\\/]/u,
              priority: 25
            },
            // FontAwesome — every consumer imports specific icons
            // (per CLAUDE.md `no library.add(...)` rule), so the
            // tree-shake works, but the SVG path tables for the
            // imported set still total a few hundred KB.
            {
              name: 'fa-icons',
              test: /[\\/]node_modules[\\/]@fortawesome[\\/]/u,
              priority: 20
            },
            // xterm — only the chat's `TerminalCard.vue` mounts it.
            {
              name: 'xterm',
              test: /[\\/]node_modules[\\/]@xterm[\\/]/u,
              priority: 20
            },
            // QR scanner + encoder — only the mobile pair landing
            // (`RemotePairScreen.vue`) consumes them.
            {
              name: 'qr',
              test: /[\\/]node_modules[\\/](qr-scanner|qrcode)[\\/]/u,
              priority: 20
            },
            // TanStack — query + virtual; pulled by every per-instance
            // composable.
            {
              name: 'tanstack',
              test: /[\\/]node_modules[\\/]@tanstack[\\/]/u,
              priority: 15
            },
            // Vue runtime + @vueuse + focus-trap. Long-cache vendor
            // floor — these don't churn version-to-version, and every
            // route needs them.
            {
              name: 'vue-runtime',
              test: /[\\/]node_modules[\\/](@vue|vue|@vueuse|focus-trap-vue|focus-trap|tabbable)[\\/]/u,
              priority: 10
            },
            // Everything else from node_modules — catch-all so the
            // app's own code stays in the entry chunk. **Excludes
            // shiki** via a path-wide negative lookahead. shiki
            // dynamic-imports `dist/langs/<lang>.mjs` +
            // `dist/themes/<theme>.mjs` at highlight time, but a
            // manual chunk wins over the dynamic-import boundary —
            // sweeping every grammar into `vendor` produced a 10 MB
            // sync chunk. The lookahead has to scan the FULL path
            // (not just the bit after `node_modules`) because pnpm
            // hoists into `.pnpm/<pkg>@<ver>/node_modules/<pkg>/...`
            // — the regex `node_modules[\/]` anchor lands on the
            // `.pnpm` directory, so a positional lookahead after it
            // never sees `shiki/`. Letting these paths fall through
            // to Rolldown's default chunker keeps them async (one
            // chunk per grammar / theme).
            {
              name: 'vendor',
              test: /^(?!.*[\\/](shiki|@shikijs)[\\/]).*[\\/]node_modules[\\/]/u,
              priority: 0
            }
          ]
        }
      }
    }
  }
})
