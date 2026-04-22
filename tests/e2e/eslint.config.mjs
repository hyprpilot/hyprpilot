import { configs, utils } from '@cenk1cenk2/eslint-config'

/** @type {import("eslint").Linter.Config[]} */
export default [
  ...configs.typescript,

  ...utils.configImportGroup({
    tsconfigDir: import.meta.dirname,
    tsconfig: 'tsconfig.json'
  }),

  // e2e boundary: wire methods are snake_case (`get_theme`, `session/load`),
  // and the Tauri/Playwright shim exposes `__TAURI_MOCK_LISTENERS__` style
  // globals. Relax the naming + dangling-underscore rules + don't force
  // return-type annotations on inline test helpers.
  {
    files: ['**/*.ts'],
    rules: {
      '@typescript-eslint/naming-convention': 'off',
      'no-underscore-dangle': 'off',
      '@typescript-eslint/explicit-function-return-type': 'off'
    }
  },

  {
    ignores: ['node_modules/', 'test-results/', 'playwright-report/', 'eslint.config.mjs', '.prettierrc.mjs']
  }
]
