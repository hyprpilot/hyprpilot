// Global component registrations for vue-tsc template type-checking.
// `FaIcon` is registered via `app.component('FaIcon', FontAwesomeIcon)` at
// boot; this augmentation teaches Volar to resolve `<FaIcon>` usages in
// templates without a per-file import.
import type { FontAwesomeIcon } from '@fortawesome/vue-fontawesome'

declare module '@vue/runtime-core' {
  interface GlobalComponents {
    FaIcon: typeof FontAwesomeIcon
  }
}
