import typescriptParser from '@typescript-eslint/parser'
import vueTypescript from '@cenk1cenk2/eslint-config/vue-typescript'
import vueParser from 'vue-eslint-parser'
import { utils } from '@cenk1cenk2/eslint-config'

/** @type {import("eslint").Linter.Config[]} */
export default [
  ...vueTypescript,

  // `@cenk1cenk2/eslint-config/vue-typescript` calls `createConfig({ extends: [] })`,
  // which skips the @vue/eslint-config-typescript parser-insertion path. Re-apply
  // the vue-eslint-parser + typescript-eslint script-block parser so that `<script
  // setup lang="ts">` blocks with TS syntax (interfaces, generics) lint correctly.
  {
    name: 'hyprpilot/vue-ts-parser',
    files: ['**/*.vue'],
    languageOptions: {
      parser: vueParser,
      parserOptions: {
        parser: typescriptParser,
        extraFileExtensions: ['.vue'],
        ecmaVersion: 2024,
        sourceType: 'module'
      }
    }
  },

  ...utils.configImportGroup({
    tsconfigDir: import.meta.dirname,
    tsconfig: 'tsconfig.json'
  }),

  {
    ignores: [
      'dist/',
      'node_modules/',
      'src-tauri/target/',
      'test-results/',
      'playwright-report/',
      'eslint.config.mjs',
      '.prettierrc.mjs'
    ]
  }
]
