import typescriptParser from '@typescript-eslint/parser'
import { configs, utils } from '@cenk1cenk2/eslint-config'
import vueTypescript from '@cenk1cenk2/eslint-config/vue-typescript'
import globals from 'globals'
import vueParser from 'vue-eslint-parser'

/** @type {import("eslint").Linter.Config[]} */
export default [
  // Browser + Node + ES2024 globals so DOM types (`window`, `document`,
  // `HTMLElement`, `KeyboardEvent`, `crypto`, `URL`) and Node-runtime
  // types (`process`, `Buffer`) don't trip `no-undef`. Vitest globals
  // (`describe`, `it`, `expect`) ride on the per-test override below.
  {
    name: 'hyprpilot/globals',
    languageOptions: {
      globals: {
        ...globals.browser,
        ...globals.node,
        ...globals.es2024
      }
    }
  },

  // `@cenk1cenk2/eslint-config/vue-typescript` only ships `.vue`-scoped
  // blocks (Vue plugin essentials + <script setup lang="ts"> parser).
  // `.ts` source files need the standalone `configs.typescript` block —
  // without it eslint matches no config for any pure-TS file and silently
  // exits, leaving the entire TypeScript codebase unlinted.
  ...configs.typescript,

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

  // `@stylistic` rule overrides addressing genuine conflicts with the
  // upstream defaults, NOT a wholesale "prettier owns formatting"
  // disable. ESLint runs after prettier in `format` and is the
  // authoritative formatter; these are surgical fixes for rules that
  // either mangle valid syntax or contradict another upstream rule.
  {
    name: 'hyprpilot/stylistic-overrides',
    rules: {
      // Allow `///` as a line-comment marker so the codebase's
      // triple-slash JSDoc-style comments survive eslint --fix
      // instead of being mangled into `// /`. `markers: ['/']` is
      // the documented escape hatch.
      'stylistic/spaced-comment': [
        'error',
        'always',
        {
          line: { markers: ['/'], exceptions: ['-', '+', '/'] },
          block: { balanced: true, markers: ['*'], exceptions: ['*'] }
        }
      ],

      // The upstream rule's `{ prev: ['case', 'default'], next: '*' }`
      // entry forces a blank line after every `case` clause. Auto-fix
      // inserts the blank line, which then trips `no-fallthrough` on
      // consecutive empty cases (`case 'a':\n\ncase 'b':\n  return …`).
      // Drop that one entry; keep the other padding rules.
      'stylistic/padding-line-between-statements': [
        'error',
        { blankLine: 'always', prev: '*', next: 'return' },
        { blankLine: 'always', prev: ['const', 'let', 'var'], next: '*' },
        { blankLine: 'any', prev: ['const', 'let', 'var'], next: ['const', 'let', 'var'] },
        { blankLine: 'always', prev: '*', next: ['function', 'if', 'try', 'break', 'class', 'for', 'while', 'do'] }
      ]
    }
  },

  // Project-specific tweaks to the upstream rule set, scoped to TS
  // files so the upstream `@typescript-eslint` plugin registration
  // (which runs on the same globs) flows through.
  {
    name: 'hyprpilot/typescript-overrides',
    files: ['**/*.ts', '**/*.tsx', '**/*.mts', '**/*.cts', '*.ts', '*.tsx', '*.mts', '*.cts'],
    rules: {
      // Naming conventions extending the upstream defaults. Adds:
      //  - enum-member `PascalCase` (codebase uses `ToolType.Bash`).
      //  - function-name `leadingUnderscore: 'allow'` for the
      //    `__resetXForTests` test seeders that several composables
      //    expose as a production-side hook (per CLAUDE.md these
      //    should eventually move to `tests/<feature>/`).
      //  - property + objectLiteralProperty `snake_case` /
      //    `UPPER_CASE` for wire-type fields mirroring Rust shapes
      //    (`session_id`, `tool_call_id`, `bash_id`, `_meta`).
      '@typescript-eslint/naming-convention': [
        'error',
        { selector: 'default', format: ['camelCase', 'PascalCase'] },
        { selector: 'variable', modifiers: ['const'], format: ['camelCase', 'UPPER_CASE', 'PascalCase'] },
        { selector: 'variable', format: ['camelCase'] },
        { selector: 'function', format: ['camelCase', 'PascalCase'], leadingUnderscore: 'allowSingleOrDouble' },
        { selector: 'parameter', format: ['camelCase', 'PascalCase'], modifiers: ['unused'], leadingUnderscore: 'require' },
        { selector: 'parameter', format: ['camelCase', 'PascalCase'], leadingUnderscore: 'allow' },
        { selector: 'property', format: ['camelCase', 'UPPER_CASE', 'snake_case', 'PascalCase'], leadingUnderscore: 'allow' },
        { selector: 'objectLiteralProperty', format: ['camelCase', 'UPPER_CASE', 'snake_case', 'PascalCase'], leadingUnderscore: 'allow' },
        // Vue/test selectors use kebab-case attribute names —
        // `data-testid`, `aria-*`, etc. — that map to object-literal
        // properties at the JS level. Allow the kebab-case shape on
        // properties whose name carries a `-`.
        { selector: 'objectLiteralProperty', filter: { regex: '-', match: true }, format: null },
        // Vite path-aliases (`@assets`, `@components`, …) appear as
        // object-literal keys in the vite/vitest configs. The `@`
        // prefix doesn't fit any standard format.
        { selector: 'objectLiteralProperty', filter: { regex: '^@', match: true }, format: null },
        { selector: 'memberLike', modifiers: ['private'], format: ['camelCase'] },
        { selector: 'enumMember', format: ['UPPER_CASE', 'camelCase', 'PascalCase'] },
        { selector: 'typeLike', format: ['PascalCase'] }
      ],

      // Codebase patterns: `_meta` (ACP wire field), `__resetXForTests`
      // (test-only seeders that CLAUDE.md flags for migration to
      // `tests/`; allowed here as transitional debt), `_adapter`
      // (intentionally unused arg). The double-underscore pattern is
      // the codebase's "this is test-internal, don't import in prod"
      // marker — disable the rule for double-underscore names; single
      // leading-underscore stays guarded.
      'no-underscore-dangle': 'off',

      // Match the codebase's `_adapter` / `_event` / `_payload`
      // intentionally-unused-arg pattern.
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_', ignoreRestSiblings: true }],

      // Object-literal method shorthand (`onCommit: () => {}`,
      // `loadingStatus: () => '…'`) and inline arrow callbacks
      // inherit their return type from the surrounding object's
      // typed shape. Annotating each is redundant churn. Standalone
      // exported functions still need a return type — that's where
      // the type contract lives.
      '@typescript-eslint/explicit-function-return-type': [
        'error',
        {
          allowExpressions: true,
          allowTypedFunctionExpressions: true,
          allowHigherOrderFunctions: true,
          allowDirectConstAssertionInArrowFunctions: true,
          allowConciseArrowFunctionExpressionsStartingWithVoid: true
        }
      ]
    }
  },

  // Same patterns for the base `no-unused-vars` (the rule still
  // applies in `.vue` `<script setup>` blocks where the upstream
  // typescript variant doesn't reach).
  {
    name: 'hyprpilot/no-unused-vars-args',
    rules: {
      'no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_', ignoreRestSiblings: true }]
    }
  },

  // shadcn-vue copy-paste components (`src/components/ui/`) carry
  // long single-line Tailwind class strings (200-500 chars) by design.
  // Reformatting them to fit `max-len` would diverge from shadcn-vue's
  // upstream source and complicate future re-syncs. Allow long lines
  // in that subtree only.
  {
    name: 'hyprpilot/shadcn-max-len',
    files: ['**/components/ui/**'],
    rules: {
      'stylistic/max-len': 'off'
    }
  },

  // Tests: pull in Vitest globals (`describe`, `it`, `expect`, `vi`).
  // Skip `explicit-function-return-type` — fixture builders and
  // `it(...)` callbacks return inline-typed object literals or
  // implicit `void`; the annotation churn outweighs the safety.
  {
    name: 'hyprpilot/test-overrides',
    files: ['**/*.test.ts', '**/*.test.tsx', '**/*.spec.ts', '**/*.spec.tsx'],
    languageOptions: {
      globals: {
        ...globals.vitest
      }
    },
    rules: {
      '@typescript-eslint/explicit-function-return-type': 'off',
      '@typescript-eslint/no-explicit-any': 'off'
    }
  },

  {
    ignores: ['dist/', 'node_modules/', 'src-tauri/target/', 'test-results/', 'playwright-report/', 'eslint.config.mjs', '.prettierrc.mjs']
  }
]
