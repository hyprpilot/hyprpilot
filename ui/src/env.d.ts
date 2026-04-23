// Vue SFC module shim: vue-tsc's Volar plugin does not activate with the
// current @volar/typescript + TypeScript 6 combination, so the bare tsc
// pass inside vue-tsc emits TS2307 for every *.vue import. This shim
// provides the minimal type information tsc needs to resolve those imports;
// vue-tsc still performs full template type-checking on the .vue source when
// the language plugin loads correctly (tracked upstream in
// vuejs/language-tools#4120 / volarjs/volar.js#189).
declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent
  export default component
}

