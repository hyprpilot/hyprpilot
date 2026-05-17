# Changelog

## [2.0.0](https://github.com/hyprpilot/hyprpilot/compare/v1.0.0...v2.0.0) (2026-05-17)


### ⚠ BREAKING CHANGES

* **v1.x:** defaults.toml no longer seeds a `[[profiles]]` entry. Fresh installs MUST supply a profile (existing captain configs already did). Daemon's `validate_profiles_non_empty` surfaces a captain-readable error at boot.

### Bug Fixes

* **v1.x:** scroll/title/cwd/aur/mobile/seeded-profile/session-order + empty thought + remote stream hoist + queue snapshot ([#89](https://github.com/hyprpilot/hyprpilot/issues/89)) ([56bdc01](https://github.com/hyprpilot/hyprpilot/commit/56bdc0184eebb077b33250220f48ee8ee8d9ceb9))

## [1.0.0](https://github.com/hyprpilot/hyprpilot/compare/v0.7.2...v1.0.0) (2026-05-17)


### ⚠ BREAKING CHANGES

* **config:** Root-level `system_prompt`, `mcps`, and `mcp` config fields are removed. Captains using them must migrate to `[[patches]]`. See CLAUDE.md's "Root-level `[[patches]]`" section for the new shape + migration examples. Defaults.toml ships one seeded patch enabling the in-tree mcp server + the XDG skills dir so fresh installs work out of the box.
* **ui:** First 1.x release — locks in the daemon's wire contract (JSON-RPC over the unix socket, the WS remote bridge, every `acp:*` Tauri event, the `[mcp]` config block, skills exposed as `hyprpilot://skills/<slug>` MCP resources) and the markdown-paragraph fold semantics for streamed agent chunks. No actual breakage in this commit — semantic versioning bump to mark stability of the surfaces shipped across the 0.x line.

### style

* **ui:** match GitHub markdown spacing in chat bubbles ([#85](https://github.com/hyprpilot/hyprpilot/issues/85)) ([674085b](https://github.com/hyprpilot/hyprpilot/commit/674085b25645fb2883b58add11e360bff667e67a))


### Features

* **daemon:** host skills as an in-tree hyprpilot MCP server ([#81](https://github.com/hyprpilot/hyprpilot/issues/81)) ([8ca3dd8](https://github.com/hyprpilot/hyprpilot/commit/8ca3dd81b6c33d40c39f58c70179c4b354e418f6))


### Bug Fixes

* **daemon:** paragraph break around tool calls + checklist stats on plans ([#83](https://github.com/hyprpilot/hyprpilot/issues/83)) ([ee6606f](https://github.com/hyprpilot/hyprpilot/commit/ee6606f38ed23bd10b81b845aceeb118fb4a7f83))
* **ui:** cancel pending sticky rAF on upward scroll so PageUp/wheel respond mid-snap ([#87](https://github.com/hyprpilot/hyprpilot/issues/87)) ([d1daf63](https://github.com/hyprpilot/hyprpilot/commit/d1daf63540fe955bed2f747b252bd4c0f84abdc0))
* **ui:** reliable chat viewport scrolling via mouse, scrollbar, and Page keys ([#84](https://github.com/hyprpilot/hyprpilot/issues/84)) ([29f328c](https://github.com/hyprpilot/hyprpilot/commit/29f328c9f313f57fd4e960b945298ea67afc116f))
* **ui:** stop palette row accent borders from leaking past rounded corners ([#86](https://github.com/hyprpilot/hyprpilot/issues/86)) ([f82f881](https://github.com/hyprpilot/hyprpilot/commit/f82f88122f4085bc4c40dcaeb8920db5006d758d))


### Refactor

* **config:** replace root-level profile-fallback fields with [[patches]] ([#88](https://github.com/hyprpilot/hyprpilot/issues/88)) ([7233929](https://github.com/hyprpilot/hyprpilot/commit/72339290fc3c7bde4299e62c26c55f74cf5fbb5c))

## [0.7.2](https://github.com/hyprpilot/hyprpilot/compare/v0.7.1...v0.7.2) (2026-05-16)


### Bug Fixes

* **daemon:** force paragraph break on messageId switch between agent chunks ([#79](https://github.com/hyprpilot/hyprpilot/issues/79)) ([3aae6fd](https://github.com/hyprpilot/hyprpilot/commit/3aae6fde9d998de80d331c4b7623d0b513ff93e3))

## [0.7.1](https://github.com/hyprpilot/hyprpilot/compare/v0.7.0...v0.7.1) (2026-05-16)


### Bug Fixes

* **daemon:** ship absolute cwd on instance wire shapes, not display form ([#77](https://github.com/hyprpilot/hyprpilot/issues/77)) ([81e207a](https://github.com/hyprpilot/hyprpilot/commit/81e207a289caf6d4bbbd3bd2a969cb5e5098137a))

## [0.7.0](https://github.com/hyprpilot/hyprpilot/compare/v0.6.0...v0.7.0) (2026-05-15)


### Features

* **daemon:** paragraph lift for single-leading \n + ship cwd on instance wire shapes ([#75](https://github.com/hyprpilot/hyprpilot/issues/75)) ([710dce6](https://github.com/hyprpilot/hyprpilot/commit/710dce64e729b46e09b798b383d2baf382229630))

## [0.6.0](https://github.com/hyprpilot/hyprpilot/compare/v0.5.0...v0.6.0) (2026-05-15)


### Features

* composer + transcript + header chrome polish ([#64](https://github.com/hyprpilot/hyprpilot/issues/64)) ([b80024d](https://github.com/hyprpilot/hyprpilot/commit/b80024db62792071e39ec4b5dea8dab7147d8604))
* **daemon:** markdown-paragraph lift on streamed agent chunks + contract docs refresh ([#73](https://github.com/hyprpilot/hyprpilot/issues/73)) ([6e1f510](https://github.com/hyprpilot/hyprpilot/commit/6e1f5108513c46ea5428b7f595e3f36662ec5c22))
* **header:** instances button redesign + queue cancel contract docs ([#66](https://github.com/hyprpilot/hyprpilot/issues/66)) ([566fd5b](https://github.com/hyprpilot/hyprpilot/commit/566fd5b518c8d79f6c95da2f6fceb55514008be1))
* internalize queue mechanism — daemon-owned, RPC-driven ([#67](https://github.com/hyprpilot/hyprpilot/issues/67)) ([5fbe25f](https://github.com/hyprpilot/hyprpilot/commit/5fbe25f324afc2c00c81614c1e1ae40feb38a66f))
* **mcps:** accept inline mcp_servers on `[[mcps]]` entries ([#61](https://github.com/hyprpilot/hyprpilot/issues/61)) ([bd9ff6b](https://github.com/hyprpilot/hyprpilot/commit/bd9ff6b54520d541c549b8a590d66441a469ad0c))
* **overlay:** accept instanceId on overlay/toggle ([#60](https://github.com/hyprpilot/hyprpilot/issues/60)) ([6e7f39b](https://github.com/hyprpilot/hyprpilot/commit/6e7f39bf573d29cc652d4f6c9340924284107193))
* **tray + pickers:** status tooltip, native cwd picker, broader attachments ([#74](https://github.com/hyprpilot/hyprpilot/issues/74)) ([6af5b28](https://github.com/hyprpilot/hyprpilot/commit/6af5b28c0baf57ba4bf87ee89b9ad46812c81caa))


### Bug Fixes

* empty-turn diagnostics + drop daemon-side path/body truncation ([#65](https://github.com/hyprpilot/hyprpilot/issues/65)) ([54c6a64](https://github.com/hyprpilot/hyprpilot/commit/54c6a64782df7e7702e910c42e84449ca493de44))
* **remote-pair:** scroll the pair landing on short phones ([#63](https://github.com/hyprpilot/hyprpilot/issues/63)) ([adcd621](https://github.com/hyprpilot/hyprpilot/commit/adcd621e390b6ea4b572fde801346381678aa099))
* **remote:** hoist transcript patcher to singleton + un-truncate header pills ([#70](https://github.com/hyprpilot/hyprpilot/issues/70)) ([77b61da](https://github.com/hyprpilot/hyprpilot/commit/77b61da4a14beaf67f57081f26fae82b8d0f3fd5))
* **submit:** let the daemon mint the instance id, read it off the reply ([#72](https://github.com/hyprpilot/hyprpilot/issues/72)) ([5027041](https://github.com/hyprpilot/hyprpilot/commit/5027041822100bf152cf5c2a35ab3f1b342c9fc5))
* **ui:** bundle warnings + agent-text fold + mobile chrome + load-older gate ([#69](https://github.com/hyprpilot/hyprpilot/issues/69)) ([47efcfb](https://github.com/hyprpilot/hyprpilot/commit/47efcfb961eef169cab423fa0b44f2967aabfef1))
* **ui:** full-history hydration + title-driven instance rows ([#71](https://github.com/hyprpilot/hyprpilot/issues/71)) ([844b7b0](https://github.com/hyprpilot/hyprpilot/commit/844b7b084adcb87f12cac42c8af304fbefbed2fe))
* update styles ([#62](https://github.com/hyprpilot/hyprpilot/issues/62)) ([ace4337](https://github.com/hyprpilot/hyprpilot/commit/ace4337d05ef285be8909efc4443e884a96148fb))
* **viewport:** clean remount per instance + gate scroll-driven pagination ([#68](https://github.com/hyprpilot/hyprpilot/issues/68)) ([cf58dcb](https://github.com/hyprpilot/hyprpilot/commit/cf58dcb4f8ef88458ee9aceaefc3659d13753077))


### Refactor

* **config:** scope withConfig patches to the resolved profile ([#58](https://github.com/hyprpilot/hyprpilot/issues/58)) ([d9ae583](https://github.com/hyprpilot/hyprpilot/commit/d9ae583eefea15105f98b877e811e9e640b60bac))

## [0.5.0](https://github.com/hyprpilot/hyprpilot/compare/v0.4.0...v0.5.0) (2026-05-14)


### Features

* **config:** accept .toml/.json/.yaml on-disk config + profile files ([#55](https://github.com/hyprpilot/hyprpilot/issues/55)) ([4b5b406](https://github.com/hyprpilot/hyprpilot/commit/4b5b406b1659ad77e6e319812f68a88bcb8b1b70))
* **sessions:** accept withConfig on session-resume ([#57](https://github.com/hyprpilot/hyprpilot/issues/57)) ([4d87f3c](https://github.com/hyprpilot/hyprpilot/commit/4d87f3c35753e3cb499c0a1e2972f9faf16e5a53))

## [0.4.0](https://github.com/hyprpilot/hyprpilot/compare/v0.3.0...v0.4.0) (2026-05-14)


### Features

* **config:** add --with-config overlay patches for per-spawn config ([#50](https://github.com/hyprpilot/hyprpilot/issues/50)) ([cd76602](https://github.com/hyprpilot/hyprpilot/commit/cd76602ce0ebeb162a3f00f18e5b09ef229e0d4a))
* **formatter:** add plain git-diff field alongside Shiki-marker description ([#44](https://github.com/hyprpilot/hyprpilot/issues/44)) ([ddffdc4](https://github.com/hyprpilot/hyprpilot/commit/ddffdc4edf67f74508143f41be8ced77af01d689))
* **permissions:** add feedback field on reject — synthesize follow-up turn ([#53](https://github.com/hyprpilot/hyprpilot/issues/53)) ([9e25f53](https://github.com/hyprpilot/hyprpilot/commit/9e25f53648f40a8602173f6e6756d8c7ca6ecbb9))
* **rpc:** lift sessions/* + completion/* onto public namespaces ([#41](https://github.com/hyprpilot/hyprpilot/issues/41)) ([a451111](https://github.com/hyprpilot/hyprpilot/commit/a451111df0fb51391e31115655204cf333201ad0))
* spawn-time header prefill + permission default option id ([#52](https://github.com/hyprpilot/hyprpilot/issues/52)) ([9531be2](https://github.com/hyprpilot/hyprpilot/commit/9531be2460222786c5886d0403e7277c41cdda45))


### Bug Fixes

* **completion:** show label at natural width, truncate detail to fit ([#39](https://github.com/hyprpilot/hyprpilot/issues/39)) ([cf55220](https://github.com/hyprpilot/hyprpilot/commit/cf552202289535f1cca1617d80f50a776f2f5868))
* **markdown:** wrap long lines in code blocks instead of horizontal-scroll ([#47](https://github.com/hyprpilot/hyprpilot/issues/47)) ([44d1ffa](https://github.com/hyprpilot/hyprpilot/commit/44d1ffa9b7e4cc4e23a737fc09adb77923bc042f))
* **modal:** floor title + permission buttons so long titles don't crush actions ([#42](https://github.com/hyprpilot/hyprpilot/issues/42)) ([7c5acf0](https://github.com/hyprpilot/hyprpilot/commit/7c5acf0d54965168266f74279d6abeb81e097166))
* **permissions:** route plan-exit to modal even when adapter is unresolved ([#43](https://github.com/hyprpilot/hyprpilot/issues/43)) ([cea22d2](https://github.com/hyprpilot/hyprpilot/commit/cea22d2213647de3d56f21ea419062b7d94b59dc))
* **ui:** defer chat-viewport eviction outside the scroll event task ([#48](https://github.com/hyprpilot/hyprpilot/issues/48)) ([7a61a46](https://github.com/hyprpilot/hyprpilot/commit/7a61a4692b54bf8a47f912f6e55e5fa03d93ddb0))
* update sizing ([#49](https://github.com/hyprpilot/hyprpilot/issues/49)) ([b23e3ba](https://github.com/hyprpilot/hyprpilot/commit/b23e3ba19851a27802d670e5604817e92a63abc9))

## [0.3.0](https://github.com/hyprpilot/hyprpilot/compare/v0.2.1...v0.3.0) (2026-05-11)


### Features

* **rpc:** accept attachments on prompts/send ([#37](https://github.com/hyprpilot/hyprpilot/issues/37)) ([2662eda](https://github.com/hyprpilot/hyprpilot/commit/2662edaf75d4576148411a75ce1ddc543679b46c))
* **rpc:** expose mode/model/option setters on instances namespace ([#36](https://github.com/hyprpilot/hyprpilot/issues/36)) ([87c2de6](https://github.com/hyprpilot/hyprpilot/commit/87c2de6b050bf6d8c2ef0160e72f1adb7c90369d))

## [0.2.1](https://github.com/hyprpilot/hyprpilot/compare/v0.2.0...v0.2.1) (2026-05-10)


### Bug Fixes

* **adapters:** cwd "no sessions" + backend-owned path display ([#34](https://github.com/hyprpilot/hyprpilot/issues/34)) ([bb97049](https://github.com/hyprpilot/hyprpilot/commit/bb97049701266232f603d3c3c75fa3e8fb1d23d9))

## [0.2.0](https://github.com/hyprpilot/hyprpilot/compare/v0.1.9...v0.2.0) (2026-05-09)


### Features

* **rpc:** events/subscribe + chore(ci): fix release-please version bumps ([#31](https://github.com/hyprpilot/hyprpilot/issues/31)) ([81d7bec](https://github.com/hyprpilot/hyprpilot/commit/81d7bec228aae04e2b6357e0f741246e077fb5be))

## [0.1.9](https://github.com/hyprpilot/hyprpilot/compare/v0.1.8...v0.1.9) (2026-05-09)


### Features

* daemon-owned transcript truth + windowed UI viewport (state replay + perf) ([#26](https://github.com/hyprpilot/hyprpilot/issues/26)) ([546400b](https://github.com/hyprpilot/hyprpilot/commit/546400b47647bf3bbc49599db3ff3f77885a0706))
* **ui:** render internal-tool output via MarkdownBody (Bash et al.) ([#30](https://github.com/hyprpilot/hyprpilot/issues/30)) ([6685fe3](https://github.com/hyprpilot/hyprpilot/commit/6685fe336eddf5534b6bb9fcee84ff72ac77c239))

## [0.1.8](https://github.com/hyprpilot/hyprpilot/compare/v0.1.7...v0.1.8) (2026-05-07)


### Features

* **remote:** TLS HTTPS+WS bridge with pair-on-connect handshake ([#22](https://github.com/hyprpilot/hyprpilot/issues/22)) ([71f6d2e](https://github.com/hyprpilot/hyprpilot/commit/71f6d2e254616cd4a8c3da2fcb4f3ffae862a18a))
* **sessions:** --restore flag + drop misleading agent/profile preview ([#20](https://github.com/hyprpilot/hyprpilot/issues/20)) ([4c57044](https://github.com/hyprpilot/hyprpilot/commit/4c57044551c208bdd2d79664ab39ad0d1df67388))
* **system-prompt:** array-of-tables shape with per-entry inject toggles + chat banner ([#21](https://github.com/hyprpilot/hyprpilot/issues/21)) ([b44bc3b](https://github.com/hyprpilot/hyprpilot/commit/b44bc3bfec60b2454cc45f60cd4ff3a5cb3d9e1b))


### Bug Fixes

* **palette:** session-restore inherits the active profile ([#19](https://github.com/hyprpilot/hyprpilot/issues/19)) ([5fb568c](https://github.com/hyprpilot/hyprpilot/commit/5fb568c0c20b3c240f75175fb667c63c5f82e565))
* **palette:** split active vs focus borders to opposite sides ([#17](https://github.com/hyprpilot/hyprpilot/issues/17)) ([b46b1bd](https://github.com/hyprpilot/hyprpilot/commit/b46b1bd26c7fe03fc3bcda44d6f9178bde542be2))


### Refactor

* **skills:** per-instance SkillsRegistry, drop daemon-global cache ([#25](https://github.com/hyprpilot/hyprpilot/issues/25)) ([083812d](https://github.com/hyprpilot/hyprpilot/commit/083812d1d8078879de2cb2ebb9c2bb7d72b9f6b6))

## [0.1.7](https://github.com/hyprpilot/hyprpilot/compare/v0.1.6...v0.1.7) (2026-05-07)


### Features

* **config:** array-of-tables shape for mcps + skills with glob ignore; ([#15](https://github.com/hyprpilot/hyprpilot/issues/15)) ([490ef9c](https://github.com/hyprpilot/hyprpilot/commit/490ef9c07485febf116c91c9a408f45165f73cd0))

## [0.1.6](https://github.com/hyprpilot/hyprpilot/compare/v0.1.5...v0.1.6) (2026-05-07)


### Features

* **keymap:** global Ctrl+F focuses the active input ([#13](https://github.com/hyprpilot/hyprpilot/issues/13)) ([2c37b31](https://github.com/hyprpilot/hyprpilot/commit/2c37b314240c7607ee90708751feefb462b8d1f5))

## [0.1.5](https://github.com/hyprpilot/hyprpilot/compare/v0.1.4...v0.1.5) (2026-05-07)


### Features

* **palette:** cwd leaf prewarms instance on empty registry ([#10](https://github.com/hyprpilot/hyprpilot/issues/10)) ([33e6d09](https://github.com/hyprpilot/hyprpilot/commit/33e6d0946f2b98cc6810eedcc975852041b8612e))


### Bug Fixes

* **chat:** tool body renders description before fields ([#12](https://github.com/hyprpilot/hyprpilot/issues/12)) ([7684d82](https://github.com/hyprpilot/hyprpilot/commit/7684d82ff5cf0be9b79ca20106b51f51ca51993c))

## [0.1.4](https://github.com/hyprpilot/hyprpilot/compare/v0.1.3...v0.1.4) (2026-05-06)


### Features

* **docs:** vitepress documentation site + tagged-release trigger fix ([#6](https://github.com/hyprpilot/hyprpilot/issues/6)) ([cb499f9](https://github.com/hyprpilot/hyprpilot/commit/cb499f9439467c4a1d07e17e34215419d6b7717b))


### Bug Fixes

* **ci:** release-please uses GITHUB_TOKEN + workflow_dispatch downstream ([#7](https://github.com/hyprpilot/hyprpilot/issues/7)) ([47b301d](https://github.com/hyprpilot/hyprpilot/commit/47b301ddd30e104f3dfd0ecc63c5e1d785552a2b))


### Documentation

* end-user tone sweep across the docs site ([#8](https://github.com/hyprpilot/hyprpilot/issues/8)) ([09c4e6f](https://github.com/hyprpilot/hyprpilot/commit/09c4e6f794b87100dc9b6a2a9b064ec0b2b9a71b))

## [0.1.3](https://github.com/hyprpilot/hyprpilot/compare/v0.1.2...v0.1.3) (2026-05-06)


### Features

* **acp+config+palette:** inline-tasks batch — out-of-turn turns, system prompt as attachment, root system_prompt, instance &gt; new ([0ec6e6d](https://github.com/hyprpilot/hyprpilot/commit/0ec6e6df19dfe7d0e3249b14c371c321d3af0057))
* **acp+config+palette:** inline-tasks batch — out-of-turn turns, system prompt as attachment, root system_prompt, instance &gt; new ([1c049c9](https://github.com/hyprpilot/hyprpilot/commit/1c049c93039f7be7cd0537a67a1c505b5f757498))
* **acp:** AcpPermissions — profile allowlists + per-request allow/deny ([afc957c](https://github.com/hyprpilot/hyprpilot/commit/afc957c738036f1a90b55bd57ca5b949f6a61653))
* **acp:** AcpPermissions — profile allowlists + per-request allow/deny ([3badc30](https://github.com/hyprpilot/hyprpilot/commit/3badc30e554d5f4cd63cee51190a4dafdf836b99))
* **acp:** advertise fs+terminal capabilities and implement handlers ([926b056](https://github.com/hyprpilot/hyprpilot/commit/926b05649e68d8bb97ec63185b6f204be028e753))
* **acp:** advertise fs+terminal capabilities and implement handlers ([16da715](https://github.com/hyprpilot/hyprpilot/commit/16da7155cae64eacf43da9164f95b4cc1aa518e2))
* **acp:** bridge daemon JSON-RPC to a coding agent via ACP (scaffold) ([9027b54](https://github.com/hyprpilot/hyprpilot/commit/9027b54a412c6babfc6aa9f2dfa848e5af30b3f9))
* **acp:** live session runtime — spawn, driver, Tauri commands + events ([0cfc3b9](https://github.com/hyprpilot/hyprpilot/commit/0cfc3b98b5ed642ad1a8f1cfaf1228269a276d49))
* **acp:** live session runtime — spawn, driver, Tauri commands + events ([472eaa5](https://github.com/hyprpilot/hyprpilot/commit/472eaa5a7d6c5a4636db0d56810bc015021daf50))
* **acp:** scaffold ACP bridge module + permission fallback chain resolver ([79b1c49](https://github.com/hyprpilot/hyprpilot/commit/79b1c4936889b0d55ac840b4d9da161f8aea2367))
* **acp:** tee subprocess stdout through tracing for wire-level debug ([3190187](https://github.com/hyprpilot/hyprpilot/commit/31901875a696018cedae2defd424bbbb0949017b))
* **acp:** UUID instance keys; multiple of same profile supported ([540d34f](https://github.com/hyprpilot/hyprpilot/commit/540d34f413da77f1947cdd29f5d24b68074b0d3f))
* **acp:** wire session/list + session/load through ACP native RPCs ([cb1cb73](https://github.com/hyprpilot/hyprpilot/commit/cb1cb73625ecf5b80ed41334c0ebe3c0ae1415e3))
* **acp:** wire session/list + session/load through ACP native RPCs ([4b36cda](https://github.com/hyprpilot/hyprpilot/commit/4b36cda66b068c97aa5b4d8cbddfa6e45216cc6d))
* backend-driven presentation (paths + ranker + tool formatters) ([3ce9bff](https://github.com/hyprpilot/hyprpilot/commit/3ce9bff0c3069b0a5827aa69fe38b4e35da09f3d))
* backend-driven presentation (paths + ranker + tool formatters) ([33a49de](https://github.com/hyprpilot/hyprpilot/commit/33a49defc768e9516bc55853fb36408c29bc0adf))
* **composer:** caret-anchored autocomplete with daemon-side sources ([825b04b](https://github.com/hyprpilot/hyprpilot/commit/825b04b724d749fa5c1d029c4bd984e4c952f6ab))
* **composer:** caret-anchored autocomplete with daemon-side sources ([5ca0087](https://github.com/hyprpilot/hyprpilot/commit/5ca008756b8e006f6566cce7f83b26c6e7837285))
* **config:** [[profiles]] with per-profile system_prompt + model overrides ([f8d2820](https://github.com/hyprpilot/hyprpilot/commit/f8d2820de0205275e67d9125ee9b19ea5abbfbfa))
* **config:** [[profiles]] with per-profile system_prompt + model overrides ([312002f](https://github.com/hyprpilot/hyprpilot/commit/312002f7133392ea7d394fb66f064aaefd2cc927))
* **config:** [keymaps] config tree — typed Rust source, collision-validated ([3b9d92b](https://github.com/hyprpilot/hyprpilot/commit/3b9d92bb766abc912cd62a29f8814d9b8cdbf2b1))
* **config:** [keymaps] config tree — typed Rust source, collision-validated ([b096189](https://github.com/hyprpilot/hyprpilot/commit/b096189720f5697320944ec6c8865cdfbee43a05))
* **config:** per-agent model field with per-vendor translation ([095a60f](https://github.com/hyprpilot/hyprpilot/commit/095a60f7350d9d8c455064e39dda212503bfa788))
* **config:** per-agent model field with per-vendor translation ([621a042](https://github.com/hyprpilot/hyprpilot/commit/621a0424a7dcce93c007f7263179dd4f65aa35e6))
* **config:** per-profile mcps / skills / mode / cwd / env / system_prompt ([e5c19f2](https://github.com/hyprpilot/hyprpilot/commit/e5c19f2b561a25f87be251c65690ef62e73ef370))
* **config:** per-profile mcps / skills / mode / cwd / env / system_prompt ([501ffb7](https://github.com/hyprpilot/hyprpilot/commit/501ffb7863390c91b3cd16ed6c817c2ae71f4738))
* **config:** seed agents registry + AcpPermissionPolicy into config layering ([220803c](https://github.com/hyprpilot/hyprpilot/commit/220803c6f6abc9e3f307c95bbd36d55fdffa9381))
* **core:** ctl parity — sessions forget + session-info ([7a57939](https://github.com/hyprpilot/hyprpilot/commit/7a57939ec6c7908af6799f1aec41f45d96942c2e))
* **core:** MCP catalog + socket RPC — global catalog, per-profile enabled set ([043ea47](https://github.com/hyprpilot/hyprpilot/commit/043ea47146b808cac757e00d17245af14b6f7e96))
* **core:** MCP catalog + socket RPC — global catalog, per-profile enabled set ([b1fea74](https://github.com/hyprpilot/hyprpilot/commit/b1fea742efe96093fdb96842b71d705f9cbe0032))
* **core:** sessions/* RPC namespace — list / info / forget ([93e4035](https://github.com/hyprpilot/hyprpilot/commit/93e40351c7d201a2acc90734f8b8edb0afa31756))
* **core:** skills loader + socket RPC + #{skill/name} expansion ([bd9c909](https://github.com/hyprpilot/hyprpilot/commit/bd9c9091b755d3f9bda4f8dc5ebecf866ece3278))
* **core:** skills loader + socket RPC + #{skill/name} expansion ([414686a](https://github.com/hyprpilot/hyprpilot/commit/414686adf2b04aaf3c19baef83bd37bae601349a))
* **core:** socket daemon + diag endpoints — status/version/reload/shutdown + snapshot ([f397393](https://github.com/hyprpilot/hyprpilot/commit/f397393e2a168431ec1ea4c532b1a76a03c7e3d9))
* **core:** socket daemon + diag endpoints — status/version/reload/shutdown + snapshot ([d7b5ec2](https://github.com/hyprpilot/hyprpilot/commit/d7b5ec20dab23e4acdbbfb9c53dcb69c1b959a2b))
* **core:** socket event subscription — events/subscribe + fanout + scoped topics ([3a0fa25](https://github.com/hyprpilot/hyprpilot/commit/3a0fa2565780b8575d5ee2e748569c28573ce9c9))
* **core:** socket event subscription — events/subscribe + fanout + scoped topics ([3ea6189](https://github.com/hyprpilot/hyprpilot/commit/3ea6189f688b818f26dfe161edd783e9bdc133f2))
* **core:** socket overlay control — present/hide/toggle (for hyprland binding) ([6cfb112](https://github.com/hyprpilot/hyprpilot/commit/6cfb112d494c373e9cd26d571a0720dae83d6768))
* **core:** socket overlay control — present/hide/toggle (for hyprland binding) ([fa7e8d0](https://github.com/hyprpilot/hyprpilot/commit/fa7e8d034d8ead9ebd316c5bfebd533a7fdee45d))
* **core:** socket passthroughs — profiles/agents/commands + modes/models per-instance ([7321afb](https://github.com/hyprpilot/hyprpilot/commit/7321afba84bf54d85aec0353e29a29888a2ecb17))
* **core:** socket passthroughs — profiles/agents/commands + modes/models per-instance ([cf54fc5](https://github.com/hyprpilot/hyprpilot/commit/cf54fc504c8d807c6c424b75696e7f373e491f86))
* **core:** socket prompts + permissions — send/cancel + pending/respond ([455cb2c](https://github.com/hyprpilot/hyprpilot/commit/455cb2c722bdfb8a68d9988e0efd87a907fec76e))
* **core:** socket prompts + permissions — send/cancel + pending/respond ([d0d8c92](https://github.com/hyprpilot/hyprpilot/commit/d0d8c92681c9ca53410e6c7b48f2ded350fc0ba5))
* **ctl:** active-instance fallback, rename, overlay/show, auto-spawn ([4d828d4](https://github.com/hyprpilot/hyprpilot/commit/4d828d402f5be5cda0656c83efab4f9a9b62fd52))
* **ctl:** active-instance fallback, rename, overlay/show, auto-spawn ([0023172](https://github.com/hyprpilot/hyprpilot/commit/0023172ece95cfd6212d35021734a2792b6fca30))
* **ctl:** prompts send --draft stages prompt into composer ([2a1a862](https://github.com/hyprpilot/hyprpilot/commit/2a1a8621d9996e802c390e349648e7e521e292e7))
* **ctl:** prompts send --draft stages prompt into composer ([759ba85](https://github.com/hyprpilot/hyprpilot/commit/759ba85e8c6d8c4f3560e5b3c57027ece7ce2203))
* **ctl:** sessions list / info / forget subcommands ([d4da51a](https://github.com/hyprpilot/hyprpilot/commit/d4da51aaf762a8b5fa218e6ce253dd5b13d4346e))
* **ctl:** waybar integration via ctl status JSON stream ([7d5eebf](https://github.com/hyprpilot/hyprpilot/commit/7d5eebf0e7a4b4090bb6ebba10965d8a970bbb61))
* **ctl:** waybar integration via ctl status JSON stream ([1c66149](https://github.com/hyprpilot/hyprpilot/commit/1c661490bd3ed9d12a02cef94cab5d6d83617f12))
* **daemon:** anchor window via zwlr_layer_shell_v1 with center fallback ([ca968e6](https://github.com/hyprpilot/hyprpilot/commit/ca968e6d055cfc7307f45a6f733728e358eacc2b))
* **daemon:** anchor window via zwlr_layer_shell_v1 with center fallback ([6a28916](https://github.com/hyprpilot/hyprpilot/commit/6a28916e61440f22a0986800f0a24a2e8a91f81a))
* **daemon:** autostart plugin + system tray + hidden-by-default boot ([19e94ea](https://github.com/hyprpilot/hyprpilot/commit/19e94ead65460865a8d620a580d88ad7f082c143))
* **daemon:** autostart plugin + system tray + hidden-by-default boot ([da0b06d](https://github.com/hyprpilot/hyprpilot/commit/da0b06db7ad79076d9ffe7287c593317fbb23452))
* **daemon:** default anchor to 40% width, full-height fill ([d37c5b9](https://github.com/hyprpilot/hyprpilot/commit/d37c5b91e4faa72d4393439942cb8249395fdb34))
* **daemon:** full-height overlay default, 40% width, percentage anchor dimensions ([ae8fad0](https://github.com/hyprpilot/hyprpilot/commit/ae8fad00fc1b8c3c341cd7b31180fa879ab1e392))
* **daemon:** wire SIGINT + SIGTERM through clean shutdown orchestrator ([b0c67a3](https://github.com/hyprpilot/hyprpilot/commit/b0c67a3b6207428f69c1407a7a4948c8f5c7fa74))
* **formatter:** tool-call stats — Vec&lt;Stat&gt; wire shape + per-stat mini-pills ([d15dfa3](https://github.com/hyprpilot/hyprpilot/commit/d15dfa36ef1c9b543d760d96fb2c525237f62c9f))
* **formatter:** tool-call stats — Vec&lt;Stat&gt; wire shape + per-stat mini-pills ([57af771](https://github.com/hyprpilot/hyprpilot/commit/57af77104e90e7cfc036267d1ca67896e7843a4b))
* **keymaps+queue+tray:** captain-driven approvals, queue dispatch, tray toggle-only ([3e8ff69](https://github.com/hyprpilot/hyprpilot/commit/3e8ff6988299c32be37ded1d55504205e30a1012))
* **keymaps+queue+tray:** captain-driven approvals, queue dispatch, tray toggle-only ([ae3f29e](https://github.com/hyprpilot/hyprpilot/commit/ae3f29e1ed667de551c96c7419867b6830564296))
* **mcp+permissions:** JSON-file MCP config + unified PermissionController pipeline + MCP tool UI fixes ([abe7e32](https://github.com/hyprpilot/hyprpilot/commit/abe7e32856675ebc8af8790dc4fdd956a9735f0b))
* **mcp+permissions:** JSON-file MCP config + unified PermissionController pipeline + MCP tool UI fixes ([ba96c24](https://github.com/hyprpilot/hyprpilot/commit/ba96c2428240eeecdc67a36f0500b152f2543612))
* **rpc:** explicit shutdown orchestration for daemon/kill ([9f9fa26](https://github.com/hyprpilot/hyprpilot/commit/9f9fa26c0cf275369502bd074ef7d127a190d8b3))
* **rpc:** JSON-RPC 2.0 over the daemon socket + ctl wiring ([0ee500f](https://github.com/hyprpilot/hyprpilot/commit/0ee500f7612fa228cd34e9fad3db82583bd26878))
* **rpc:** JSON-RPC 2.0 over the daemon socket + ctl wiring ([5bb2076](https://github.com/hyprpilot/hyprpilot/commit/5bb2076763f18eef8c785c2afc013aed769b654b))
* **scaffold:** bootstrap Cargo + Tauri 2 + Vue 3 + shadcn-vue repo ([705fe7e](https://github.com/hyprpilot/hyprpilot/commit/705fe7e151bd2cab93590a6b0530e58ec6c8e459))
* **scaffold:** bootstrap Cargo + Tauri 2 + Vue 3 + shadcn-vue repo ([7f7fcc3](https://github.com/hyprpilot/hyprpilot/commit/7f7fcc322d9c330b68b54b9bb03d9a11f7180724))
* **ui:** chat view — transcript, composer, profile switcher, session list ([3235981](https://github.com/hyprpilot/hyprpilot/commit/32359812c80eeae56060233126c4e30e8bff4fe2))
* **ui:** chat view — transcript, composer, profile switcher, session list ([e024b9e](https://github.com/hyprpilot/hyprpilot/commit/e024b9e9eb2dad36ff2836bc973d7963a81e9ddb))
* **ui:** ChatBody renders agent text through markdown pipeline ([9deeded](https://github.com/hyprpilot/hyprpilot/commit/9deeded1bd2d8e0641774e35b027cccb13d51a1b))
* **ui:** command palette primitive — recursive overlay, fuzzy filter, multi/select modes, stub root leaves ([b8a8b3e](https://github.com/hyprpilot/hyprpilot/commit/b8a8b3e587b9d4b717d1c2b8e121a70f8b30ec0d))
* **ui:** command palette primitive — recursive overlay, fuzzy filter, multi/select modes, stub root leaves ([e5550bb](https://github.com/hyprpilot/hyprpilot/commit/e5550bb17716f5facb368c37f51222eb5df2c0a7))
* **ui:** composer state — pills, token expansion, Ctrl+P clipboard image paste ([7a74fe3](https://github.com/hyprpilot/hyprpilot/commit/7a74fe3a476f02b3c102525b0c604fba91ac0c78))
* **ui:** composer state — pills, token expansion, Ctrl+P clipboard image paste ([4e14326](https://github.com/hyprpilot/hyprpilot/commit/4e1432609e5ac7797d0e3de67b58146c0471f625))
* **ui:** design primitives from Claude wireframe bundle — tokens, chrome, chat, command-palette, screen fixtures + Chat.vue migration ([e565fb6](https://github.com/hyprpilot/hyprpilot/commit/e565fb643f4e7b52be59286b57e84e8628e5ff73))
* **ui:** design primitives from Claude wireframe bundle — tokens, chrome, chat, command-palette, screen fixtures + Chat.vue migration ([6cd5ab3](https://github.com/hyprpilot/hyprpilot/commit/6cd5ab34310ce2ff0b090adf5ec5f69ed703bc10))
* **ui:** header wiring — SessionInfoUpdate title + breadcrumbs row ([d96352b](https://github.com/hyprpilot/hyprpilot/commit/d96352b2d0a6d2764b3520b3f3bc4dba9fa5c7ba))
* **ui:** header wiring — SessionInfoUpdate title + breadcrumbs row ([97c64c4](https://github.com/hyprpilot/hyprpilot/commit/97c64c45697e15d0e232872a279d36f4a9fe8b1e))
* **ui:** inline terminal card wiring — terminal/output + terminal/wait_for_exit streaming ([34ad5c0](https://github.com/hyprpilot/hyprpilot/commit/34ad5c0e056477b981cd91fec6a02bc97ba440e4))
* **ui:** inline terminal card wiring — terminal/output + terminal/wait_for_exit streaming ([277228f](https://github.com/hyprpilot/hyprpilot/commit/277228f3bd585e1a447377158dc3fa453ee53158))
* **ui:** markdown + Shiki per-codeblock rendering in agent output ([4b6ef46](https://github.com/hyprpilot/hyprpilot/commit/4b6ef4640f939a36224e16ed17c86de8a3d9f731))
* **ui:** markdown renderer — markdown-it + Shiki + DOMPurify ([bd6fe4c](https://github.com/hyprpilot/hyprpilot/commit/bd6fe4c6e5d281797df64ae0c2a9e83965559c3a))
* **ui:** overlay shell rename — Chat.vue → Overlay.vue ([591356f](https://github.com/hyprpilot/hyprpilot/commit/591356f0170d683ad4ea3ad3fb432a82ff813f39))
* **ui:** overlay shell rename — Chat.vue → Overlay.vue ([1c0eb31](https://github.com/hyprpilot/hyprpilot/commit/1c0eb31fe0e8b4faea0cc70198dbf0cc262cf102))
* **ui:** paint window-edge accent on inward side of the overlay ([d846df4](https://github.com/hyprpilot/hyprpilot/commit/d846df474a34162918742733f1839393e9c3f89d))
* **ui:** paint window-edge accent on the inward side of the overlay ([688d263](https://github.com/hyprpilot/hyprpilot/commit/688d263e7519b11818737208c8adf130adb15e62))
* **ui:** palette leaf — commands (insert slash-name into composer) ([78bcb2f](https://github.com/hyprpilot/hyprpilot/commit/78bcb2feeb14425e931afe12243e894847ac3dd5))
* **ui:** palette leaf — commands (insert slash-name into composer) ([10372ea](https://github.com/hyprpilot/hyprpilot/commit/10372ead3cde080b2abf9883d5650972f4c84f12))
* **ui:** palette leaf — cwd (single-select) with session restart on change ([41fecd5](https://github.com/hyprpilot/hyprpilot/commit/41fecd5e513eff609e8b481690017b5047e68d80))
* **ui:** palette leaf — cwd (single-select) with session restart on change ([fadc806](https://github.com/hyprpilot/hyprpilot/commit/fadc8067e81872370d50daafff5a1e4ca5618b96))
* **ui:** palette leaf — instances (single-select, focus/shutdown) + active-instance store ([48af4c0](https://github.com/hyprpilot/hyprpilot/commit/48af4c0780137e041076209bfdd3827bf37d93db))
* **ui:** palette leaf — instances (single-select, focus/shutdown) + active-instance store ([fd348e3](https://github.com/hyprpilot/hyprpilot/commit/fd348e33ba8b0b634a6d6348b9ec40d2474ef2e9))
* **ui:** palette leaf — MCPs (multi-select) with session restart on change ([2836536](https://github.com/hyprpilot/hyprpilot/commit/28365364098bfe0d7e912dc31c457f89459af215))
* **ui:** palette leaf — MCPs (multi-select) with session restart on change ([9252199](https://github.com/hyprpilot/hyprpilot/commit/9252199d594b3e53bf217f3432951013afaac53e))
* **ui:** palette leaf — profiles (single-select) ([bc7e623](https://github.com/hyprpilot/hyprpilot/commit/bc7e623a58e3ac625ee9c85289106826c555046d))
* **ui:** palette leaf — profiles (single-select) ([96f78a5](https://github.com/hyprpilot/hyprpilot/commit/96f78a5cf89927c3b27f8fa271186c1fdf701be3))
* **ui:** palette leaf — sessions with preview + Ctrl+D delete + session/load ([fd194b1](https://github.com/hyprpilot/hyprpilot/commit/fd194b1c9f930842bc0745764fffdc0e4efa0638))
* **ui:** palette leaf — sessions with preview + Ctrl+D delete + session/load ([7c9914a](https://github.com/hyprpilot/hyprpilot/commit/7c9914a76141ef0451d727aef4ca855a5b80cb29))
* **ui:** palette leaf — skills (multi-select) ([1a4fdf9](https://github.com/hyprpilot/hyprpilot/commit/1a4fdf9421a7b8f2e2d2efb27bb7a29e003508e1))
* **ui:** palette leaf — skills (multi-select) ([9e97578](https://github.com/hyprpilot/hyprpilot/commit/9e97578af1f9b38c063ac0ff9dcd723edaa6df2c))
* **ui:** palette leaves — models + modes (single-select each) ([fd20f67](https://github.com/hyprpilot/hyprpilot/commit/fd20f670a7133924658572ff8ca687980ca45d94))
* **ui:** palette leaves — models + modes (single-select each) ([12667cb](https://github.com/hyprpilot/hyprpilot/commit/12667cbb97b3e8f8cc72a9d496014c04fa1c2322))
* **ui:** permission reply wiring — PermissionStack allow/deny → permission_reply ([03f0fb3](https://github.com/hyprpilot/hyprpilot/commit/03f0fb3a45c4f294f6c00b48b31988d3c6c7b7ae))
* **ui:** permission reply wiring — PermissionStack allow/deny → permission_reply ([e817f62](https://github.com/hyprpilot/hyprpilot/commit/e817f624dadbfe4c7b4722c36f650ca0aeee00a1))
* **ui:** phase state machine — per-instance idle/working/streaming/pending/awaiting ([1b6c4f9](https://github.com/hyprpilot/hyprpilot/commit/1b6c4f9bd9c0df7b153d9d39f45c3c67bf4809d1))
* **ui:** phase state machine — per-instance idle/working/streaming/pending/awaiting ([8149722](https://github.com/hyprpilot/hyprpilot/commit/81497228ea0fa93217424d8fd9244555655caa9a))
* **ui:** queue state machine — FIFO above composer, dispatch on turn_complete ([3260e21](https://github.com/hyprpilot/hyprpilot/commit/3260e21a54157eaebf3b3b1dc8a7ca99555879e8))
* **ui:** queue state machine — FIFO above composer, dispatch on turn_complete ([ba9e115](https://github.com/hyprpilot/hyprpilot/commit/ba9e115d9820c71810aa61f1d4677b6b214b5791))
* **ui:** session event demux — per-primitive typed stores keyed by instance id ([8c202dc](https://github.com/hyprpilot/hyprpilot/commit/8c202dc52c635789979e098617615b3e7b3763fa))
* **ui:** session event demux — per-primitive typed stores keyed by instance id ([eb3812d](https://github.com/hyprpilot/hyprpilot/commit/eb3812dcc0cbb394f173605afb623736e81b4742))
* **ui:** tauri-plugin-log bridge + lib/log.ts + instrument user actions ([336f916](https://github.com/hyprpilot/hyprpilot/commit/336f9161becacfee103fa45cf6a082867921a97a))
* **ui:** tauri-plugin-log bridge + lib/log.ts + instrument user actions ([b1a3ee5](https://github.com/hyprpilot/hyprpilot/commit/b1a3ee5f48dadac65fc7046c818b970dfb36e7bd))
* **ui:** toast row driver — 4s transient status stack ([b7bb947](https://github.com/hyprpilot/hyprpilot/commit/b7bb94725ee93510cd387b34d954f8fb74890e77))
* **ui:** toast row driver — 4s transient status stack ([0c26aa0](https://github.com/hyprpilot/hyprpilot/commit/0c26aa0ce36641ebf0a621581d029f3bdcf204f7))


### Bug Fixes

* **acp:** bump subprocess shutdown grace window 2s → 5s ([3770a41](https://github.com/hyprpilot/hyprpilot/commit/3770a419b5e88c0d8e6e9068a722e7920857f979))
* **acp:** enable unstable feature for newer SessionUpdate variants ([8018af4](https://github.com/hyprpilot/hyprpilot/commit/8018af47475b1d4e04680c14aa277763bf640cac))
* **acp:** enable unstable feature for newer SessionUpdate variants ([66abe5f](https://github.com/hyprpilot/hyprpilot/commit/66abe5f427e74a0811411903e89f9a4c9fb7edfc))
* **acp:** mirror configOptions mode/model flips into RwLocks ([7e1c032](https://github.com/hyprpilot/hyprpilot/commit/7e1c032689921d6aadf4c6b4a883ba20c40557ad))
* **acp:** mirror configOptions mode/model flips into RwLocks ([fb120ea](https://github.com/hyprpilot/hyprpilot/commit/fb120ea66f3f3b580fe5342429b009198d6dad63))
* **acp:** race child.wait() against connect_with for fast dead-child detection ([06827c9](https://github.com/hyprpilot/hyprpilot/commit/06827c9ac9a6324040e17887231dadb538ec9830))
* **acp:** universal synthetic-turn close timer (queue-stuck regression) ([df87d49](https://github.com/hyprpilot/hyprpilot/commit/df87d497827c51320994b1a82a87f29a0eb25244))
* **acp:** universal synthetic-turn close timer (queue-stuck regression) ([7229b65](https://github.com/hyprpilot/hyprpilot/commit/7229b652ab14edd58327667ce397f8484aa392b4))
* **adapters:** address K-251 MR !34 review ([93ae531](https://github.com/hyprpilot/hyprpilot/commit/93ae5313a69dbea9030bc97fadbf8d146d551ace))
* **aur:** branch pkgver() on captured git describe output ([99a2efb](https://github.com/hyprpilot/hyprpilot/commit/99a2efb06b13cef194d1dde1fb9162c1d0468a07))
* **aur:** cargo build --locked instead of --frozen ([55e2958](https://github.com/hyprpilot/hyprpilot/commit/55e29588ed7cd280996fc5970d671041b78567f8))
* **aur:** point lfs.url at GitHub HTTPS so makepkg can pull LFS objects ([650598a](https://github.com/hyprpilot/hyprpilot/commit/650598a8da43a85eacb9bba7b491d8a33d14a344))
* **aur:** pull LFS objects in prepare() so icons are real PNGs ([f6c35c6](https://github.com/hyprpilot/hyprpilot/commit/f6c35c65488d5cb9829690c7435a9676d70c21c0))
* **aur:** suppress git describe exit code under makepkg set -e ([5514476](https://github.com/hyprpilot/hyprpilot/commit/5514476e199202db732a00f34aad070528ad3f42))
* **ci:** hydrate Git LFS on checkout + jsdom-safe scrollIntoView ([f900753](https://github.com/hyprpilot/hyprpilot/commit/f90075313c83ee5401a493fe4802d98b649939d4))
* **ci:** hydrate Git LFS on checkout + jsdom-safe scrollIntoView ([a01b756](https://github.com/hyprpilot/hyprpilot/commit/a01b7569410dfe6b1c2c0c115763382db2eeacc4))
* **ci:** trigger release.yml on `release: published`, not tag push ([#4](https://github.com/hyprpilot/hyprpilot/issues/4)) ([4c769c0](https://github.com/hyprpilot/hyprpilot/commit/4c769c0e682ab6b8121d3a536443cf90d73356f9))
* cleanup a bit ([37cd273](https://github.com/hyprpilot/hyprpilot/commit/37cd273ab54894f9cd5ba3b5a0c367c6f12d1cf2))
* cleanup further ([a37b8b0](https://github.com/hyprpilot/hyprpilot/commit/a37b8b00da127da2cbb7839cfb729e1e45079b64))
* **composer:** keep empty-textarea inline height unset ([cf0c69c](https://github.com/hyprpilot/hyprpilot/commit/cf0c69c39edde4f4c0990c1c28ca1a75818ce241))
* **composer:** keep empty-textarea inline height unset ([33bbf69](https://github.com/hyprpilot/hyprpilot/commit/33bbf69be815ab352ffa64eb1c2681b11040d528))
* **composer:** pipe active-instance cwd into autocomplete query ([4534e94](https://github.com/hyprpilot/hyprpilot/commit/4534e94a3fb50482f7ba923a0c3fd1765786673b))
* **composer:** pipe active-instance cwd into autocomplete query ([a55bd95](https://github.com/hyprpilot/hyprpilot/commit/a55bd95d765350cee6f2ea28d79bc2bb9b3e3b3b))
* **core:** close TerminalStream enum — squash-merge dropped brace ([f4cc8e7](https://github.com/hyprpilot/hyprpilot/commit/f4cc8e785f84d83360cefefd66598c0426a50423))
* **core:** close TerminalStream enum — squash-merge dropped brace ([befc143](https://github.com/hyprpilot/hyprpilot/commit/befc1432ec9a182bae3750b4f92472535be635ca))
* **core:** resolve tracing subscriber double-install at startup ([9af7e8a](https://github.com/hyprpilot/hyprpilot/commit/9af7e8a635a464c3ef4db0ec920226b192e4a0a4))
* **daemon:** resolve anchor width/height against current monitor on every show ([e90d06e](https://github.com/hyprpilot/hyprpilot/commit/e90d06e8b753726e9e4fd504e7dcf0f6ef650845))
* **daemon:** resolve anchor width/height against current monitor on every show ([0e071c6](https://github.com/hyprpilot/hyprpilot/commit/0e071c6bae94081cbcffe0264edc7e25f9846654))
* **deps:** update dependency @vueuse/core to v14 ([1a5a6e8](https://github.com/hyprpilot/hyprpilot/commit/1a5a6e878f28989eafb4437a4d32e197b0ada832))
* **deps:** update dependency @vueuse/core to v14 ([923dd32](https://github.com/hyprpilot/hyprpilot/commit/923dd325dc20d4bfa8a837b89cd6eea6b43c397c))
* **deps:** update dependency shiki to v4 ([8a5eac1](https://github.com/hyprpilot/hyprpilot/commit/8a5eac10be6ea8c16fabc986618a8b90cfa3861b))
* **deps:** update dependency shiki to v4 ([d984c50](https://github.com/hyprpilot/hyprpilot/commit/d984c500d9d0ff7a655c31baf6fea895ea3620aa))
* **mcps+effort:** per-instance MCP catalog + config-option banner ([521920f](https://github.com/hyprpilot/hyprpilot/commit/521920fc39427882a2edd47284370b0459b07daf))
* **mcps+effort:** per-instance MCP catalog + config-option banner ([1b7782f](https://github.com/hyprpilot/hyprpilot/commit/1b7782f26f259bad870212b04a706eff8e428ba2))
* paths_resolve flat args + thread cwd through session_load ([4baeb86](https://github.com/hyprpilot/hyprpilot/commit/4baeb86a24b0e4ccbe14c2bc3a5cf8f1280302ab))
* paths_resolve flat args + thread cwd through session_load ([e1bd571](https://github.com/hyprpilot/hyprpilot/commit/e1bd571bb54c35dfff1d041e99e43e6642880220))
* **permissions:** Ctrl+G / Ctrl+R bind only to basic-once variants ([98dd222](https://github.com/hyprpilot/hyprpilot/commit/98dd222a65860ef792d0d13cc27f65ea6e1ff525))
* **permissions:** Ctrl+G / Ctrl+R bind only to basic-once variants ([5f3bdc8](https://github.com/hyprpilot/hyprpilot/commit/5f3bdc8cea9c7165f67712fe2cfaa2dfbd159afd))
* queue stuck + claude-code thought extraction + user-msg markdown + composer 50vh ([3b76f5e](https://github.com/hyprpilot/hyprpilot/commit/3b76f5eab9c2330cd55594aa2fcd89a43f189688))
* queue stuck + claude-code thought extraction + user-msg markdown + composer 50vh ([45c871f](https://github.com/hyprpilot/hyprpilot/commit/45c871f9f85cb35fd664fe273a2d6437cc4b38b4))
* **sessions:** track focused-instance profile, not picker selection ([c948357](https://github.com/hyprpilot/hyprpilot/commit/c9483578ecba4b9fe156c1a18f07a0634d4425d8))
* subprocess stderr capture, tool-call success color, per-turn grouping ([74fecf0](https://github.com/hyprpilot/hyprpilot/commit/74fecf08da58422701ff04de15f80be046b23033))
* **terminal:** drain pipe readers before wait() returns ([1d3763f](https://github.com/hyprpilot/hyprpilot/commit/1d3763f39023f40e3f867dd75a6cce4d493c87da))
* udpate binary ([6d841f6](https://github.com/hyprpilot/hyprpilot/commit/6d841f6adfac77839e69fa926c979f078c038ea5))
* **ui:** MarkdownBody — collapse the giant gap on GFM task-list rows ([a846672](https://github.com/hyprpilot/hyprpilot/commit/a84667289443089b892a07a046702a9bc9c95631))
* **ui:** plan modal must overlay viewport, not the scrolled chat region ([b8dcede](https://github.com/hyprpilot/hyprpilot/commit/b8dcedeb71ae7faff1221fed2f0888161f7a006a))
* **ui:** push user turn before submit invoke so it wins the seq race ([58c01b1](https://github.com/hyprpilot/hyprpilot/commit/58c01b14d11e3b32401db5f0aab91a377aeb6ac2))
* **ui:** resolve pre-existing vue-tsc --noEmit failures ([5870752](https://github.com/hyprpilot/hyprpilot/commit/58707520d2de80bf768685f394d289495c5f8d3f))
* **ui:** resolve pre-existing vue-tsc --noEmit failures ([9a30bf6](https://github.com/hyprpilot/hyprpilot/commit/9a30bf6eaa7c403a434b751987040f300661a7b9))
* **ui:** tighten modal trigger + diagnostic for missing permission prompts ([d9f8a44](https://github.com/hyprpilot/hyprpilot/commit/d9f8a4464c78fa5b7e1fa551c2cbba25c238b507))
* update arguments ([872acf7](https://github.com/hyprpilot/hyprpilot/commit/872acf7a3f04050e38552dd36a9324d99313aa59))
* update basic configuration ([5403e09](https://github.com/hyprpilot/hyprpilot/commit/5403e09896a78db96bfa928a99ec934ae148ce5e))
* update defaults ([0484144](https://github.com/hyprpilot/hyprpilot/commit/048414429d915005ddbe3b92a3b95f367a2804fc))
* update defaults ([f37bd1f](https://github.com/hyprpilot/hyprpilot/commit/f37bd1f0f99904dc318d625a2ef3afffe4a7cc99))
* update dependency ([71d9852](https://github.com/hyprpilot/hyprpilot/commit/71d9852c98dd6cff35d6874150f01de9bb331400))


### Refactor

* **acp:** drop AcpPermissionPolicy — PermissionController is separate scope ([9b4c567](https://github.com/hyprpilot/hyprpilot/commit/9b4c5679a80d041f7fb0dae4637c6d85e767cde2))
* **adapters:** relocate Tauri commands to the generic layer ([95df7d5](https://github.com/hyprpilot/hyprpilot/commit/95df7d5159a0b148285d37dc9aae9c2340a6df16))
* **adapters:** relocate Tauri commands to the generic layer ([83c3cbb](https://github.com/hyprpilot/hyprpilot/commit/83c3cbbab378e460370913fb73f05e59489477d8))
* **adapters:** typed transcript pipeline + actor lifecycle on AcpInstance ([af6b853](https://github.com/hyprpilot/hyprpilot/commit/af6b8536b806ceed84341252de3628f7df6f4ec5))
* **adapters:** typed transcript pipeline + actor lifecycle on AcpInstance ([2b8b939](https://github.com/hyprpilot/hyprpilot/commit/2b8b939dfb40c55d1d48b4c1c956598b46173737))
* backend refinement batch (12 cleanups) ([e85fab1](https://github.com/hyprpilot/hyprpilot/commit/e85fab1eccec46046372427a7103a2b6750f6675))
* backend refinement batch (12 cleanups) ([e034707](https://github.com/hyprpilot/hyprpilot/commit/e0347079226cc80be13d8e9b04c4e41529a6cffc))
* bullshit-detection audit — 15 of 20 opportunities ([90d65d0](https://github.com/hyprpilot/hyprpilot/commit/90d65d0edae455f751d03823ab1359aafab6ca11))
* bullshit-detection audit — 15 of 20 opportunities ([578de66](https://github.com/hyprpilot/hyprpilot/commit/578de6660101154d64d88ee1fbc08918f71edcdc))
* clean the handlers for commands ([2051781](https://github.com/hyprpilot/hyprpilot/commit/205178105f3c3ee6df1c61e96fe9aa9cb17c64ae))
* cleanup round 2 — permission transparency, wire-title honesty, shared chrome ([dee636d](https://github.com/hyprpilot/hyprpilot/commit/dee636d9138778488b92af42390b18c32e869375))
* cleanup round 2 — permission transparency, wire-title honesty, shared chrome ([cad89d8](https://github.com/hyprpilot/hyprpilot/commit/cad89d8f661df8a3db753881afd58048a73be5fb))
* **config:** adopt merge crate, fold validators into garde, split mod.rs ([f99fcaa](https://github.com/hyprpilot/hyprpilot/commit/f99fcaa0a5bdc3f5385ceb985eda3d562003847e))
* **config:** adopt merge crate, fold validators into garde, split mod.rs ([8557370](https://github.com/hyprpilot/hyprpilot/commit/85573700b111eac3be99dc8c8f1340d00117cd4a))
* **config:** introduce HexColor newtype for theme colour fields ([60cb2bb](https://github.com/hyprpilot/hyprpilot/commit/60cb2bb773f37c78a1fc3bad0b8a0f64da01702a))
* **config:** Logging.level uses logging::LogLevel enum directly ([031c7c8](https://github.com/hyprpilot/hyprpilot/commit/031c7c878d2c38036b28582a87b1db83bea93c7a))
* **config:** make defaults.toml the single source of default values ([b141c6d](https://github.com/hyprpilot/hyprpilot/commit/b141c6d6ba66673cf96cbcc84b34618ab914a84b))
* **config:** make defaults.toml the single source of default values ([bfa6804](https://github.com/hyprpilot/hyprpilot/commit/bfa68042a0a2ae8dc9a90a000450151041afa41f))
* **config:** move active_agent under [agent] section ([0aacb27](https://github.com/hyprpilot/hyprpilot/commit/0aacb27aabc52b637498ab2b5dccadba7fe4b6a7))
* **config:** move validators to config/validations.rs ([b04e63f](https://github.com/hyprpilot/hyprpilot/commit/b04e63f86b5da34db83b3426dfde752a1f0d72ec))
* **config:** unify layer merging behind a Merge trait ([f524c3d](https://github.com/hyprpilot/hyprpilot/commit/f524c3d370dbf98c68a93be838baf1034920f8d6))
* **core:** adapters/ scaffold — Adapter trait + generic types ([ac8d10a](https://github.com/hyprpilot/hyprpilot/commit/ac8d10a65592991a386a98d73243a15639ae7650))
* **core:** relocate acp/ → adapters/acp/; session→instance renames; Tauri event rename ([104ecfb](https://github.com/hyprpilot/hyprpilot/commit/104ecfb490a17cb2c887f273a5b18358c6087ba7))
* **core:** src/adapters layout — Adapter trait, ACP as an impl, session→instance, Acp prefix audit ([fa0f08e](https://github.com/hyprpilot/hyprpilot/commit/fa0f08ea1e2fa01fb2a991e1e7e0b2cefdf2d51e))
* **ctl:** collapse handler boilerplate into single-match dispatch ([f4cca1f](https://github.com/hyprpilot/hyprpilot/commit/f4cca1f515c0d9ed42194db68d252ee7d1ab98cd))
* **ctl:** collapse handler boilerplate into single-match dispatch ([df288db](https://github.com/hyprpilot/hyprpilot/commit/df288dbb770104b050100c3fa66f718255227ae1))
* **daemon:** split run() + extract desktop integration ([6556b7f](https://github.com/hyprpilot/hyprpilot/commit/6556b7f99d99ef6287e8ba376b9d8d52b5d54e64))
* **daemon:** split run() + extract desktop integration ([b367b71](https://github.com/hyprpilot/hyprpilot/commit/b367b718c5e04909f6fe619409cc7fd5e78e1b7e))
* **mcp:** strip speculative broadcast; relocate MCPDefinition ([9f0a196](https://github.com/hyprpilot/hyprpilot/commit/9f0a1964555c66a4262989795a67a5b655cea609))
* **mcp:** strip speculative broadcast; relocate MCPDefinition ([06ee206](https://github.com/hyprpilot/hyprpilot/commit/06ee2063b803f77c7e2471fd05d5a5ead62788a6))
* **rpc:** prune surface, make trait load-bearing, write_line helper ([b68e408](https://github.com/hyprpilot/hyprpilot/commit/b68e40816ace27b70e2c731edfd806721fd86e94))
* **rpc:** prune surface, make trait load-bearing, write_line helper ([2ae0b0a](https://github.com/hyprpilot/hyprpilot/commit/2ae0b0a7d2dfc897d61cb83b3d3680e3e2a5902c))
* **rpc:** signal daemon shutdown via response payload, not a side-channel flag ([b1d88d7](https://github.com/hyprpilot/hyprpilot/commit/b1d88d74ec8bb5356b74666f5b62c6f2a3af5401))
* **rpc:** split CoreHandler into namespaced session/window/daemon handlers ([2c1289e](https://github.com/hyprpilot/hyprpilot/commit/2c1289e1c4a5fa2e56d21b35318cbba20a9459d0))
* skills/paths/logging cleanup — strip dead broadcast, cache BaseDirs, drop home ([d0fc77c](https://github.com/hyprpilot/hyprpilot/commit/d0fc77c53c8f93d61d2addebf29e6302be03254d))
* skills/paths/logging cleanup — strip dead broadcast, cache BaseDirs, drop home ([0e4784b](https://github.com/hyprpilot/hyprpilot/commit/0e4784bd7a5df1f0b5ba2adc62ffdde5485612ed))
* **ui-tools:** unified ToolCallView formatter + per-tool folders ([7b55674](https://github.com/hyprpilot/hyprpilot/commit/7b55674e5bd7cefcd69908449d3162e48b4b4022))
* **ui-tools:** unified ToolCallView formatter + per-tool folders ([0b6a255](https://github.com/hyprpilot/hyprpilot/commit/0b6a2555ec47a88a4093c80b38940f41dd434534))
* **ui:** D5 reskin foundation — theme, shadcn install, audit fixes ([d7b6dfd](https://github.com/hyprpilot/hyprpilot/commit/d7b6dfd06558e7e298c0947936246da181f455ba))
* **ui:** D5 reskin foundation — theme, shadcn install, audit fixes ([eeeabcb](https://github.com/hyprpilot/hyprpilot/commit/eeeabcb483cf86eb753b991f2bbf0a3175eef545))


### Documentation

* **claude-md:** document the ACP scaffold + agents config shape ([97d3f1c](https://github.com/hyprpilot/hyprpilot/commit/97d3f1ca210db69dfcbfb153b43aa8e7a0be6637))
* **claude-md:** document WindowManager adapter + client-side handler pattern ([895b272](https://github.com/hyprpilot/hyprpilot/commit/895b272c3fc23e7211651b74d4f75e05738e7812))
* **claude:** add upstream migration runway + manual verification patterns ([ee93c50](https://github.com/hyprpilot/hyprpilot/commit/ee93c50847f3c44b0909e4a213340ff6a1fbe2ff))
* **claude:** add upstream migration runway + manual verification patterns ([9782401](https://github.com/hyprpilot/hyprpilot/commit/97824012f285db9304e2ece7cf81f6504bae38ad))
* **claude:** codify composition rules from !21 review ([00ec2cb](https://github.com/hyprpilot/hyprpilot/commit/00ec2cb03d03e496ae7122b4e817b60efa74415f))
* **claude:** codify composition rules from !21 review ([4cccfb7](https://github.com/hyprpilot/hyprpilot/commit/4cccfb7e11255882cc495feb500f379ecb67d716))
* **claude:** document session_list + session_load Tauri commands ([a2b2b42](https://github.com/hyprpilot/hyprpilot/commit/a2b2b42378bbc85788e2ebb235986c62b67a50ff))
* **claude:** document session_list + session_load Tauri commands ([645f00d](https://github.com/hyprpilot/hyprpilot/commit/645f00d30b26d36c3032eccd263823e4a877cf90))
* trim oversized comments + capture deviations in CLAUDE.md ([ef01a1c](https://github.com/hyprpilot/hyprpilot/commit/ef01a1c528cca985fd2e976ef1b00a3e38266cc9))

## [0.1.2](https://github.com/hyprpilot/hyprpilot/compare/v0.1.1...v0.1.2) (2026-05-06)


### Features

* **acp+config+palette:** inline-tasks batch — out-of-turn turns, system prompt as attachment, root system_prompt, instance &gt; new ([0ec6e6d](https://github.com/hyprpilot/hyprpilot/commit/0ec6e6df19dfe7d0e3249b14c371c321d3af0057))
* **acp+config+palette:** inline-tasks batch — out-of-turn turns, system prompt as attachment, root system_prompt, instance &gt; new ([1c049c9](https://github.com/hyprpilot/hyprpilot/commit/1c049c93039f7be7cd0537a67a1c505b5f757498))
* **acp:** AcpPermissions — profile allowlists + per-request allow/deny ([afc957c](https://github.com/hyprpilot/hyprpilot/commit/afc957c738036f1a90b55bd57ca5b949f6a61653))
* **acp:** AcpPermissions — profile allowlists + per-request allow/deny ([3badc30](https://github.com/hyprpilot/hyprpilot/commit/3badc30e554d5f4cd63cee51190a4dafdf836b99))
* **acp:** advertise fs+terminal capabilities and implement handlers ([926b056](https://github.com/hyprpilot/hyprpilot/commit/926b05649e68d8bb97ec63185b6f204be028e753))
* **acp:** advertise fs+terminal capabilities and implement handlers ([16da715](https://github.com/hyprpilot/hyprpilot/commit/16da7155cae64eacf43da9164f95b4cc1aa518e2))
* **acp:** bridge daemon JSON-RPC to a coding agent via ACP (scaffold) ([9027b54](https://github.com/hyprpilot/hyprpilot/commit/9027b54a412c6babfc6aa9f2dfa848e5af30b3f9))
* **acp:** live session runtime — spawn, driver, Tauri commands + events ([0cfc3b9](https://github.com/hyprpilot/hyprpilot/commit/0cfc3b98b5ed642ad1a8f1cfaf1228269a276d49))
* **acp:** live session runtime — spawn, driver, Tauri commands + events ([472eaa5](https://github.com/hyprpilot/hyprpilot/commit/472eaa5a7d6c5a4636db0d56810bc015021daf50))
* **acp:** scaffold ACP bridge module + permission fallback chain resolver ([79b1c49](https://github.com/hyprpilot/hyprpilot/commit/79b1c4936889b0d55ac840b4d9da161f8aea2367))
* **acp:** tee subprocess stdout through tracing for wire-level debug ([3190187](https://github.com/hyprpilot/hyprpilot/commit/31901875a696018cedae2defd424bbbb0949017b))
* **acp:** UUID instance keys; multiple of same profile supported ([540d34f](https://github.com/hyprpilot/hyprpilot/commit/540d34f413da77f1947cdd29f5d24b68074b0d3f))
* **acp:** wire session/list + session/load through ACP native RPCs ([cb1cb73](https://github.com/hyprpilot/hyprpilot/commit/cb1cb73625ecf5b80ed41334c0ebe3c0ae1415e3))
* **acp:** wire session/list + session/load through ACP native RPCs ([4b36cda](https://github.com/hyprpilot/hyprpilot/commit/4b36cda66b068c97aa5b4d8cbddfa6e45216cc6d))
* backend-driven presentation (paths + ranker + tool formatters) ([3ce9bff](https://github.com/hyprpilot/hyprpilot/commit/3ce9bff0c3069b0a5827aa69fe38b4e35da09f3d))
* backend-driven presentation (paths + ranker + tool formatters) ([33a49de](https://github.com/hyprpilot/hyprpilot/commit/33a49defc768e9516bc55853fb36408c29bc0adf))
* **composer:** caret-anchored autocomplete with daemon-side sources ([825b04b](https://github.com/hyprpilot/hyprpilot/commit/825b04b724d749fa5c1d029c4bd984e4c952f6ab))
* **composer:** caret-anchored autocomplete with daemon-side sources ([5ca0087](https://github.com/hyprpilot/hyprpilot/commit/5ca008756b8e006f6566cce7f83b26c6e7837285))
* **config:** [[profiles]] with per-profile system_prompt + model overrides ([f8d2820](https://github.com/hyprpilot/hyprpilot/commit/f8d2820de0205275e67d9125ee9b19ea5abbfbfa))
* **config:** [[profiles]] with per-profile system_prompt + model overrides ([312002f](https://github.com/hyprpilot/hyprpilot/commit/312002f7133392ea7d394fb66f064aaefd2cc927))
* **config:** [keymaps] config tree — typed Rust source, collision-validated ([3b9d92b](https://github.com/hyprpilot/hyprpilot/commit/3b9d92bb766abc912cd62a29f8814d9b8cdbf2b1))
* **config:** [keymaps] config tree — typed Rust source, collision-validated ([b096189](https://github.com/hyprpilot/hyprpilot/commit/b096189720f5697320944ec6c8865cdfbee43a05))
* **config:** per-agent model field with per-vendor translation ([095a60f](https://github.com/hyprpilot/hyprpilot/commit/095a60f7350d9d8c455064e39dda212503bfa788))
* **config:** per-agent model field with per-vendor translation ([621a042](https://github.com/hyprpilot/hyprpilot/commit/621a0424a7dcce93c007f7263179dd4f65aa35e6))
* **config:** per-profile mcps / skills / mode / cwd / env / system_prompt ([e5c19f2](https://github.com/hyprpilot/hyprpilot/commit/e5c19f2b561a25f87be251c65690ef62e73ef370))
* **config:** per-profile mcps / skills / mode / cwd / env / system_prompt ([501ffb7](https://github.com/hyprpilot/hyprpilot/commit/501ffb7863390c91b3cd16ed6c817c2ae71f4738))
* **config:** seed agents registry + AcpPermissionPolicy into config layering ([220803c](https://github.com/hyprpilot/hyprpilot/commit/220803c6f6abc9e3f307c95bbd36d55fdffa9381))
* **core:** ctl parity — sessions forget + session-info ([7a57939](https://github.com/hyprpilot/hyprpilot/commit/7a57939ec6c7908af6799f1aec41f45d96942c2e))
* **core:** MCP catalog + socket RPC — global catalog, per-profile enabled set ([043ea47](https://github.com/hyprpilot/hyprpilot/commit/043ea47146b808cac757e00d17245af14b6f7e96))
* **core:** MCP catalog + socket RPC — global catalog, per-profile enabled set ([b1fea74](https://github.com/hyprpilot/hyprpilot/commit/b1fea742efe96093fdb96842b71d705f9cbe0032))
* **core:** sessions/* RPC namespace — list / info / forget ([93e4035](https://github.com/hyprpilot/hyprpilot/commit/93e40351c7d201a2acc90734f8b8edb0afa31756))
* **core:** skills loader + socket RPC + #{skill/name} expansion ([bd9c909](https://github.com/hyprpilot/hyprpilot/commit/bd9c9091b755d3f9bda4f8dc5ebecf866ece3278))
* **core:** skills loader + socket RPC + #{skill/name} expansion ([414686a](https://github.com/hyprpilot/hyprpilot/commit/414686adf2b04aaf3c19baef83bd37bae601349a))
* **core:** socket daemon + diag endpoints — status/version/reload/shutdown + snapshot ([f397393](https://github.com/hyprpilot/hyprpilot/commit/f397393e2a168431ec1ea4c532b1a76a03c7e3d9))
* **core:** socket daemon + diag endpoints — status/version/reload/shutdown + snapshot ([d7b5ec2](https://github.com/hyprpilot/hyprpilot/commit/d7b5ec20dab23e4acdbbfb9c53dcb69c1b959a2b))
* **core:** socket event subscription — events/subscribe + fanout + scoped topics ([3a0fa25](https://github.com/hyprpilot/hyprpilot/commit/3a0fa2565780b8575d5ee2e748569c28573ce9c9))
* **core:** socket event subscription — events/subscribe + fanout + scoped topics ([3ea6189](https://github.com/hyprpilot/hyprpilot/commit/3ea6189f688b818f26dfe161edd783e9bdc133f2))
* **core:** socket overlay control — present/hide/toggle (for hyprland binding) ([6cfb112](https://github.com/hyprpilot/hyprpilot/commit/6cfb112d494c373e9cd26d571a0720dae83d6768))
* **core:** socket overlay control — present/hide/toggle (for hyprland binding) ([fa7e8d0](https://github.com/hyprpilot/hyprpilot/commit/fa7e8d034d8ead9ebd316c5bfebd533a7fdee45d))
* **core:** socket passthroughs — profiles/agents/commands + modes/models per-instance ([7321afb](https://github.com/hyprpilot/hyprpilot/commit/7321afba84bf54d85aec0353e29a29888a2ecb17))
* **core:** socket passthroughs — profiles/agents/commands + modes/models per-instance ([cf54fc5](https://github.com/hyprpilot/hyprpilot/commit/cf54fc504c8d807c6c424b75696e7f373e491f86))
* **core:** socket prompts + permissions — send/cancel + pending/respond ([455cb2c](https://github.com/hyprpilot/hyprpilot/commit/455cb2c722bdfb8a68d9988e0efd87a907fec76e))
* **core:** socket prompts + permissions — send/cancel + pending/respond ([d0d8c92](https://github.com/hyprpilot/hyprpilot/commit/d0d8c92681c9ca53410e6c7b48f2ded350fc0ba5))
* **ctl:** active-instance fallback, rename, overlay/show, auto-spawn ([4d828d4](https://github.com/hyprpilot/hyprpilot/commit/4d828d402f5be5cda0656c83efab4f9a9b62fd52))
* **ctl:** active-instance fallback, rename, overlay/show, auto-spawn ([0023172](https://github.com/hyprpilot/hyprpilot/commit/0023172ece95cfd6212d35021734a2792b6fca30))
* **ctl:** prompts send --draft stages prompt into composer ([2a1a862](https://github.com/hyprpilot/hyprpilot/commit/2a1a8621d9996e802c390e349648e7e521e292e7))
* **ctl:** prompts send --draft stages prompt into composer ([759ba85](https://github.com/hyprpilot/hyprpilot/commit/759ba85e8c6d8c4f3560e5b3c57027ece7ce2203))
* **ctl:** sessions list / info / forget subcommands ([d4da51a](https://github.com/hyprpilot/hyprpilot/commit/d4da51aaf762a8b5fa218e6ce253dd5b13d4346e))
* **ctl:** waybar integration via ctl status JSON stream ([7d5eebf](https://github.com/hyprpilot/hyprpilot/commit/7d5eebf0e7a4b4090bb6ebba10965d8a970bbb61))
* **ctl:** waybar integration via ctl status JSON stream ([1c66149](https://github.com/hyprpilot/hyprpilot/commit/1c661490bd3ed9d12a02cef94cab5d6d83617f12))
* **daemon:** anchor window via zwlr_layer_shell_v1 with center fallback ([ca968e6](https://github.com/hyprpilot/hyprpilot/commit/ca968e6d055cfc7307f45a6f733728e358eacc2b))
* **daemon:** anchor window via zwlr_layer_shell_v1 with center fallback ([6a28916](https://github.com/hyprpilot/hyprpilot/commit/6a28916e61440f22a0986800f0a24a2e8a91f81a))
* **daemon:** autostart plugin + system tray + hidden-by-default boot ([19e94ea](https://github.com/hyprpilot/hyprpilot/commit/19e94ead65460865a8d620a580d88ad7f082c143))
* **daemon:** autostart plugin + system tray + hidden-by-default boot ([da0b06d](https://github.com/hyprpilot/hyprpilot/commit/da0b06db7ad79076d9ffe7287c593317fbb23452))
* **daemon:** default anchor to 40% width, full-height fill ([d37c5b9](https://github.com/hyprpilot/hyprpilot/commit/d37c5b91e4faa72d4393439942cb8249395fdb34))
* **daemon:** full-height overlay default, 40% width, percentage anchor dimensions ([ae8fad0](https://github.com/hyprpilot/hyprpilot/commit/ae8fad00fc1b8c3c341cd7b31180fa879ab1e392))
* **daemon:** wire SIGINT + SIGTERM through clean shutdown orchestrator ([b0c67a3](https://github.com/hyprpilot/hyprpilot/commit/b0c67a3b6207428f69c1407a7a4948c8f5c7fa74))
* **formatter:** tool-call stats — Vec&lt;Stat&gt; wire shape + per-stat mini-pills ([d15dfa3](https://github.com/hyprpilot/hyprpilot/commit/d15dfa36ef1c9b543d760d96fb2c525237f62c9f))
* **formatter:** tool-call stats — Vec&lt;Stat&gt; wire shape + per-stat mini-pills ([57af771](https://github.com/hyprpilot/hyprpilot/commit/57af77104e90e7cfc036267d1ca67896e7843a4b))
* **keymaps+queue+tray:** captain-driven approvals, queue dispatch, tray toggle-only ([3e8ff69](https://github.com/hyprpilot/hyprpilot/commit/3e8ff6988299c32be37ded1d55504205e30a1012))
* **keymaps+queue+tray:** captain-driven approvals, queue dispatch, tray toggle-only ([ae3f29e](https://github.com/hyprpilot/hyprpilot/commit/ae3f29e1ed667de551c96c7419867b6830564296))
* **mcp+permissions:** JSON-file MCP config + unified PermissionController pipeline + MCP tool UI fixes ([abe7e32](https://github.com/hyprpilot/hyprpilot/commit/abe7e32856675ebc8af8790dc4fdd956a9735f0b))
* **mcp+permissions:** JSON-file MCP config + unified PermissionController pipeline + MCP tool UI fixes ([ba96c24](https://github.com/hyprpilot/hyprpilot/commit/ba96c2428240eeecdc67a36f0500b152f2543612))
* **rpc:** explicit shutdown orchestration for daemon/kill ([9f9fa26](https://github.com/hyprpilot/hyprpilot/commit/9f9fa26c0cf275369502bd074ef7d127a190d8b3))
* **rpc:** JSON-RPC 2.0 over the daemon socket + ctl wiring ([0ee500f](https://github.com/hyprpilot/hyprpilot/commit/0ee500f7612fa228cd34e9fad3db82583bd26878))
* **rpc:** JSON-RPC 2.0 over the daemon socket + ctl wiring ([5bb2076](https://github.com/hyprpilot/hyprpilot/commit/5bb2076763f18eef8c785c2afc013aed769b654b))
* **scaffold:** bootstrap Cargo + Tauri 2 + Vue 3 + shadcn-vue repo ([705fe7e](https://github.com/hyprpilot/hyprpilot/commit/705fe7e151bd2cab93590a6b0530e58ec6c8e459))
* **scaffold:** bootstrap Cargo + Tauri 2 + Vue 3 + shadcn-vue repo ([7f7fcc3](https://github.com/hyprpilot/hyprpilot/commit/7f7fcc322d9c330b68b54b9bb03d9a11f7180724))
* **ui:** chat view — transcript, composer, profile switcher, session list ([3235981](https://github.com/hyprpilot/hyprpilot/commit/32359812c80eeae56060233126c4e30e8bff4fe2))
* **ui:** chat view — transcript, composer, profile switcher, session list ([e024b9e](https://github.com/hyprpilot/hyprpilot/commit/e024b9e9eb2dad36ff2836bc973d7963a81e9ddb))
* **ui:** ChatBody renders agent text through markdown pipeline ([9deeded](https://github.com/hyprpilot/hyprpilot/commit/9deeded1bd2d8e0641774e35b027cccb13d51a1b))
* **ui:** command palette primitive — recursive overlay, fuzzy filter, multi/select modes, stub root leaves ([b8a8b3e](https://github.com/hyprpilot/hyprpilot/commit/b8a8b3e587b9d4b717d1c2b8e121a70f8b30ec0d))
* **ui:** command palette primitive — recursive overlay, fuzzy filter, multi/select modes, stub root leaves ([e5550bb](https://github.com/hyprpilot/hyprpilot/commit/e5550bb17716f5facb368c37f51222eb5df2c0a7))
* **ui:** composer state — pills, token expansion, Ctrl+P clipboard image paste ([7a74fe3](https://github.com/hyprpilot/hyprpilot/commit/7a74fe3a476f02b3c102525b0c604fba91ac0c78))
* **ui:** composer state — pills, token expansion, Ctrl+P clipboard image paste ([4e14326](https://github.com/hyprpilot/hyprpilot/commit/4e1432609e5ac7797d0e3de67b58146c0471f625))
* **ui:** design primitives from Claude wireframe bundle — tokens, chrome, chat, command-palette, screen fixtures + Chat.vue migration ([e565fb6](https://github.com/hyprpilot/hyprpilot/commit/e565fb643f4e7b52be59286b57e84e8628e5ff73))
* **ui:** design primitives from Claude wireframe bundle — tokens, chrome, chat, command-palette, screen fixtures + Chat.vue migration ([6cd5ab3](https://github.com/hyprpilot/hyprpilot/commit/6cd5ab34310ce2ff0b090adf5ec5f69ed703bc10))
* **ui:** header wiring — SessionInfoUpdate title + breadcrumbs row ([d96352b](https://github.com/hyprpilot/hyprpilot/commit/d96352b2d0a6d2764b3520b3f3bc4dba9fa5c7ba))
* **ui:** header wiring — SessionInfoUpdate title + breadcrumbs row ([97c64c4](https://github.com/hyprpilot/hyprpilot/commit/97c64c45697e15d0e232872a279d36f4a9fe8b1e))
* **ui:** inline terminal card wiring — terminal/output + terminal/wait_for_exit streaming ([34ad5c0](https://github.com/hyprpilot/hyprpilot/commit/34ad5c0e056477b981cd91fec6a02bc97ba440e4))
* **ui:** inline terminal card wiring — terminal/output + terminal/wait_for_exit streaming ([277228f](https://github.com/hyprpilot/hyprpilot/commit/277228f3bd585e1a447377158dc3fa453ee53158))
* **ui:** markdown + Shiki per-codeblock rendering in agent output ([4b6ef46](https://github.com/hyprpilot/hyprpilot/commit/4b6ef4640f939a36224e16ed17c86de8a3d9f731))
* **ui:** markdown renderer — markdown-it + Shiki + DOMPurify ([bd6fe4c](https://github.com/hyprpilot/hyprpilot/commit/bd6fe4c6e5d281797df64ae0c2a9e83965559c3a))
* **ui:** overlay shell rename — Chat.vue → Overlay.vue ([591356f](https://github.com/hyprpilot/hyprpilot/commit/591356f0170d683ad4ea3ad3fb432a82ff813f39))
* **ui:** overlay shell rename — Chat.vue → Overlay.vue ([1c0eb31](https://github.com/hyprpilot/hyprpilot/commit/1c0eb31fe0e8b4faea0cc70198dbf0cc262cf102))
* **ui:** paint window-edge accent on inward side of the overlay ([d846df4](https://github.com/hyprpilot/hyprpilot/commit/d846df474a34162918742733f1839393e9c3f89d))
* **ui:** paint window-edge accent on the inward side of the overlay ([688d263](https://github.com/hyprpilot/hyprpilot/commit/688d263e7519b11818737208c8adf130adb15e62))
* **ui:** palette leaf — commands (insert slash-name into composer) ([78bcb2f](https://github.com/hyprpilot/hyprpilot/commit/78bcb2feeb14425e931afe12243e894847ac3dd5))
* **ui:** palette leaf — commands (insert slash-name into composer) ([10372ea](https://github.com/hyprpilot/hyprpilot/commit/10372ead3cde080b2abf9883d5650972f4c84f12))
* **ui:** palette leaf — cwd (single-select) with session restart on change ([41fecd5](https://github.com/hyprpilot/hyprpilot/commit/41fecd5e513eff609e8b481690017b5047e68d80))
* **ui:** palette leaf — cwd (single-select) with session restart on change ([fadc806](https://github.com/hyprpilot/hyprpilot/commit/fadc8067e81872370d50daafff5a1e4ca5618b96))
* **ui:** palette leaf — instances (single-select, focus/shutdown) + active-instance store ([48af4c0](https://github.com/hyprpilot/hyprpilot/commit/48af4c0780137e041076209bfdd3827bf37d93db))
* **ui:** palette leaf — instances (single-select, focus/shutdown) + active-instance store ([fd348e3](https://github.com/hyprpilot/hyprpilot/commit/fd348e33ba8b0b634a6d6348b9ec40d2474ef2e9))
* **ui:** palette leaf — MCPs (multi-select) with session restart on change ([2836536](https://github.com/hyprpilot/hyprpilot/commit/28365364098bfe0d7e912dc31c457f89459af215))
* **ui:** palette leaf — MCPs (multi-select) with session restart on change ([9252199](https://github.com/hyprpilot/hyprpilot/commit/9252199d594b3e53bf217f3432951013afaac53e))
* **ui:** palette leaf — profiles (single-select) ([bc7e623](https://github.com/hyprpilot/hyprpilot/commit/bc7e623a58e3ac625ee9c85289106826c555046d))
* **ui:** palette leaf — profiles (single-select) ([96f78a5](https://github.com/hyprpilot/hyprpilot/commit/96f78a5cf89927c3b27f8fa271186c1fdf701be3))
* **ui:** palette leaf — sessions with preview + Ctrl+D delete + session/load ([fd194b1](https://github.com/hyprpilot/hyprpilot/commit/fd194b1c9f930842bc0745764fffdc0e4efa0638))
* **ui:** palette leaf — sessions with preview + Ctrl+D delete + session/load ([7c9914a](https://github.com/hyprpilot/hyprpilot/commit/7c9914a76141ef0451d727aef4ca855a5b80cb29))
* **ui:** palette leaf — skills (multi-select) ([1a4fdf9](https://github.com/hyprpilot/hyprpilot/commit/1a4fdf9421a7b8f2e2d2efb27bb7a29e003508e1))
* **ui:** palette leaf — skills (multi-select) ([9e97578](https://github.com/hyprpilot/hyprpilot/commit/9e97578af1f9b38c063ac0ff9dcd723edaa6df2c))
* **ui:** palette leaves — models + modes (single-select each) ([fd20f67](https://github.com/hyprpilot/hyprpilot/commit/fd20f670a7133924658572ff8ca687980ca45d94))
* **ui:** palette leaves — models + modes (single-select each) ([12667cb](https://github.com/hyprpilot/hyprpilot/commit/12667cbb97b3e8f8cc72a9d496014c04fa1c2322))
* **ui:** permission reply wiring — PermissionStack allow/deny → permission_reply ([03f0fb3](https://github.com/hyprpilot/hyprpilot/commit/03f0fb3a45c4f294f6c00b48b31988d3c6c7b7ae))
* **ui:** permission reply wiring — PermissionStack allow/deny → permission_reply ([e817f62](https://github.com/hyprpilot/hyprpilot/commit/e817f624dadbfe4c7b4722c36f650ca0aeee00a1))
* **ui:** phase state machine — per-instance idle/working/streaming/pending/awaiting ([1b6c4f9](https://github.com/hyprpilot/hyprpilot/commit/1b6c4f9bd9c0df7b153d9d39f45c3c67bf4809d1))
* **ui:** phase state machine — per-instance idle/working/streaming/pending/awaiting ([8149722](https://github.com/hyprpilot/hyprpilot/commit/81497228ea0fa93217424d8fd9244555655caa9a))
* **ui:** queue state machine — FIFO above composer, dispatch on turn_complete ([3260e21](https://github.com/hyprpilot/hyprpilot/commit/3260e21a54157eaebf3b3b1dc8a7ca99555879e8))
* **ui:** queue state machine — FIFO above composer, dispatch on turn_complete ([ba9e115](https://github.com/hyprpilot/hyprpilot/commit/ba9e115d9820c71810aa61f1d4677b6b214b5791))
* **ui:** session event demux — per-primitive typed stores keyed by instance id ([8c202dc](https://github.com/hyprpilot/hyprpilot/commit/8c202dc52c635789979e098617615b3e7b3763fa))
* **ui:** session event demux — per-primitive typed stores keyed by instance id ([eb3812d](https://github.com/hyprpilot/hyprpilot/commit/eb3812dcc0cbb394f173605afb623736e81b4742))
* **ui:** tauri-plugin-log bridge + lib/log.ts + instrument user actions ([336f916](https://github.com/hyprpilot/hyprpilot/commit/336f9161becacfee103fa45cf6a082867921a97a))
* **ui:** tauri-plugin-log bridge + lib/log.ts + instrument user actions ([b1a3ee5](https://github.com/hyprpilot/hyprpilot/commit/b1a3ee5f48dadac65fc7046c818b970dfb36e7bd))
* **ui:** toast row driver — 4s transient status stack ([b7bb947](https://github.com/hyprpilot/hyprpilot/commit/b7bb94725ee93510cd387b34d954f8fb74890e77))
* **ui:** toast row driver — 4s transient status stack ([0c26aa0](https://github.com/hyprpilot/hyprpilot/commit/0c26aa0ce36641ebf0a621581d029f3bdcf204f7))


### Bug Fixes

* **acp:** bump subprocess shutdown grace window 2s → 5s ([3770a41](https://github.com/hyprpilot/hyprpilot/commit/3770a419b5e88c0d8e6e9068a722e7920857f979))
* **acp:** enable unstable feature for newer SessionUpdate variants ([8018af4](https://github.com/hyprpilot/hyprpilot/commit/8018af47475b1d4e04680c14aa277763bf640cac))
* **acp:** enable unstable feature for newer SessionUpdate variants ([66abe5f](https://github.com/hyprpilot/hyprpilot/commit/66abe5f427e74a0811411903e89f9a4c9fb7edfc))
* **acp:** mirror configOptions mode/model flips into RwLocks ([7e1c032](https://github.com/hyprpilot/hyprpilot/commit/7e1c032689921d6aadf4c6b4a883ba20c40557ad))
* **acp:** mirror configOptions mode/model flips into RwLocks ([fb120ea](https://github.com/hyprpilot/hyprpilot/commit/fb120ea66f3f3b580fe5342429b009198d6dad63))
* **acp:** race child.wait() against connect_with for fast dead-child detection ([06827c9](https://github.com/hyprpilot/hyprpilot/commit/06827c9ac9a6324040e17887231dadb538ec9830))
* **acp:** universal synthetic-turn close timer (queue-stuck regression) ([df87d49](https://github.com/hyprpilot/hyprpilot/commit/df87d497827c51320994b1a82a87f29a0eb25244))
* **acp:** universal synthetic-turn close timer (queue-stuck regression) ([7229b65](https://github.com/hyprpilot/hyprpilot/commit/7229b652ab14edd58327667ce397f8484aa392b4))
* **adapters:** address K-251 MR !34 review ([93ae531](https://github.com/hyprpilot/hyprpilot/commit/93ae5313a69dbea9030bc97fadbf8d146d551ace))
* **aur:** branch pkgver() on captured git describe output ([99a2efb](https://github.com/hyprpilot/hyprpilot/commit/99a2efb06b13cef194d1dde1fb9162c1d0468a07))
* **aur:** cargo build --locked instead of --frozen ([55e2958](https://github.com/hyprpilot/hyprpilot/commit/55e29588ed7cd280996fc5970d671041b78567f8))
* **aur:** point lfs.url at GitHub HTTPS so makepkg can pull LFS objects ([650598a](https://github.com/hyprpilot/hyprpilot/commit/650598a8da43a85eacb9bba7b491d8a33d14a344))
* **aur:** pull LFS objects in prepare() so icons are real PNGs ([f6c35c6](https://github.com/hyprpilot/hyprpilot/commit/f6c35c65488d5cb9829690c7435a9676d70c21c0))
* **aur:** suppress git describe exit code under makepkg set -e ([5514476](https://github.com/hyprpilot/hyprpilot/commit/5514476e199202db732a00f34aad070528ad3f42))
* **ci:** hydrate Git LFS on checkout + jsdom-safe scrollIntoView ([f900753](https://github.com/hyprpilot/hyprpilot/commit/f90075313c83ee5401a493fe4802d98b649939d4))
* **ci:** hydrate Git LFS on checkout + jsdom-safe scrollIntoView ([a01b756](https://github.com/hyprpilot/hyprpilot/commit/a01b7569410dfe6b1c2c0c115763382db2eeacc4))
* cleanup a bit ([37cd273](https://github.com/hyprpilot/hyprpilot/commit/37cd273ab54894f9cd5ba3b5a0c367c6f12d1cf2))
* cleanup further ([a37b8b0](https://github.com/hyprpilot/hyprpilot/commit/a37b8b00da127da2cbb7839cfb729e1e45079b64))
* **composer:** keep empty-textarea inline height unset ([cf0c69c](https://github.com/hyprpilot/hyprpilot/commit/cf0c69c39edde4f4c0990c1c28ca1a75818ce241))
* **composer:** keep empty-textarea inline height unset ([33bbf69](https://github.com/hyprpilot/hyprpilot/commit/33bbf69be815ab352ffa64eb1c2681b11040d528))
* **composer:** pipe active-instance cwd into autocomplete query ([4534e94](https://github.com/hyprpilot/hyprpilot/commit/4534e94a3fb50482f7ba923a0c3fd1765786673b))
* **composer:** pipe active-instance cwd into autocomplete query ([a55bd95](https://github.com/hyprpilot/hyprpilot/commit/a55bd95d765350cee6f2ea28d79bc2bb9b3e3b3b))
* **core:** close TerminalStream enum — squash-merge dropped brace ([f4cc8e7](https://github.com/hyprpilot/hyprpilot/commit/f4cc8e785f84d83360cefefd66598c0426a50423))
* **core:** close TerminalStream enum — squash-merge dropped brace ([befc143](https://github.com/hyprpilot/hyprpilot/commit/befc1432ec9a182bae3750b4f92472535be635ca))
* **core:** resolve tracing subscriber double-install at startup ([9af7e8a](https://github.com/hyprpilot/hyprpilot/commit/9af7e8a635a464c3ef4db0ec920226b192e4a0a4))
* **daemon:** resolve anchor width/height against current monitor on every show ([e90d06e](https://github.com/hyprpilot/hyprpilot/commit/e90d06e8b753726e9e4fd504e7dcf0f6ef650845))
* **daemon:** resolve anchor width/height against current monitor on every show ([0e071c6](https://github.com/hyprpilot/hyprpilot/commit/0e071c6bae94081cbcffe0264edc7e25f9846654))
* **deps:** update dependency @vueuse/core to v14 ([1a5a6e8](https://github.com/hyprpilot/hyprpilot/commit/1a5a6e878f28989eafb4437a4d32e197b0ada832))
* **deps:** update dependency @vueuse/core to v14 ([923dd32](https://github.com/hyprpilot/hyprpilot/commit/923dd325dc20d4bfa8a837b89cd6eea6b43c397c))
* **deps:** update dependency shiki to v4 ([8a5eac1](https://github.com/hyprpilot/hyprpilot/commit/8a5eac10be6ea8c16fabc986618a8b90cfa3861b))
* **deps:** update dependency shiki to v4 ([d984c50](https://github.com/hyprpilot/hyprpilot/commit/d984c500d9d0ff7a655c31baf6fea895ea3620aa))
* **mcps+effort:** per-instance MCP catalog + config-option banner ([521920f](https://github.com/hyprpilot/hyprpilot/commit/521920fc39427882a2edd47284370b0459b07daf))
* **mcps+effort:** per-instance MCP catalog + config-option banner ([1b7782f](https://github.com/hyprpilot/hyprpilot/commit/1b7782f26f259bad870212b04a706eff8e428ba2))
* paths_resolve flat args + thread cwd through session_load ([4baeb86](https://github.com/hyprpilot/hyprpilot/commit/4baeb86a24b0e4ccbe14c2bc3a5cf8f1280302ab))
* paths_resolve flat args + thread cwd through session_load ([e1bd571](https://github.com/hyprpilot/hyprpilot/commit/e1bd571bb54c35dfff1d041e99e43e6642880220))
* **permissions:** Ctrl+G / Ctrl+R bind only to basic-once variants ([98dd222](https://github.com/hyprpilot/hyprpilot/commit/98dd222a65860ef792d0d13cc27f65ea6e1ff525))
* **permissions:** Ctrl+G / Ctrl+R bind only to basic-once variants ([5f3bdc8](https://github.com/hyprpilot/hyprpilot/commit/5f3bdc8cea9c7165f67712fe2cfaa2dfbd159afd))
* queue stuck + claude-code thought extraction + user-msg markdown + composer 50vh ([3b76f5e](https://github.com/hyprpilot/hyprpilot/commit/3b76f5eab9c2330cd55594aa2fcd89a43f189688))
* queue stuck + claude-code thought extraction + user-msg markdown + composer 50vh ([45c871f](https://github.com/hyprpilot/hyprpilot/commit/45c871f9f85cb35fd664fe273a2d6437cc4b38b4))
* **sessions:** track focused-instance profile, not picker selection ([c948357](https://github.com/hyprpilot/hyprpilot/commit/c9483578ecba4b9fe156c1a18f07a0634d4425d8))
* subprocess stderr capture, tool-call success color, per-turn grouping ([74fecf0](https://github.com/hyprpilot/hyprpilot/commit/74fecf08da58422701ff04de15f80be046b23033))
* **terminal:** drain pipe readers before wait() returns ([1d3763f](https://github.com/hyprpilot/hyprpilot/commit/1d3763f39023f40e3f867dd75a6cce4d493c87da))
* udpate binary ([6d841f6](https://github.com/hyprpilot/hyprpilot/commit/6d841f6adfac77839e69fa926c979f078c038ea5))
* **ui:** MarkdownBody — collapse the giant gap on GFM task-list rows ([a846672](https://github.com/hyprpilot/hyprpilot/commit/a84667289443089b892a07a046702a9bc9c95631))
* **ui:** plan modal must overlay viewport, not the scrolled chat region ([b8dcede](https://github.com/hyprpilot/hyprpilot/commit/b8dcedeb71ae7faff1221fed2f0888161f7a006a))
* **ui:** push user turn before submit invoke so it wins the seq race ([58c01b1](https://github.com/hyprpilot/hyprpilot/commit/58c01b14d11e3b32401db5f0aab91a377aeb6ac2))
* **ui:** resolve pre-existing vue-tsc --noEmit failures ([5870752](https://github.com/hyprpilot/hyprpilot/commit/58707520d2de80bf768685f394d289495c5f8d3f))
* **ui:** resolve pre-existing vue-tsc --noEmit failures ([9a30bf6](https://github.com/hyprpilot/hyprpilot/commit/9a30bf6eaa7c403a434b751987040f300661a7b9))
* **ui:** tighten modal trigger + diagnostic for missing permission prompts ([d9f8a44](https://github.com/hyprpilot/hyprpilot/commit/d9f8a4464c78fa5b7e1fa551c2cbba25c238b507))
* update arguments ([872acf7](https://github.com/hyprpilot/hyprpilot/commit/872acf7a3f04050e38552dd36a9324d99313aa59))
* update basic configuration ([5403e09](https://github.com/hyprpilot/hyprpilot/commit/5403e09896a78db96bfa928a99ec934ae148ce5e))
* update defaults ([0484144](https://github.com/hyprpilot/hyprpilot/commit/048414429d915005ddbe3b92a3b95f367a2804fc))
* update defaults ([f37bd1f](https://github.com/hyprpilot/hyprpilot/commit/f37bd1f0f99904dc318d625a2ef3afffe4a7cc99))
* update dependency ([71d9852](https://github.com/hyprpilot/hyprpilot/commit/71d9852c98dd6cff35d6874150f01de9bb331400))


### Refactor

* **acp:** drop AcpPermissionPolicy — PermissionController is separate scope ([9b4c567](https://github.com/hyprpilot/hyprpilot/commit/9b4c5679a80d041f7fb0dae4637c6d85e767cde2))
* **adapters:** relocate Tauri commands to the generic layer ([95df7d5](https://github.com/hyprpilot/hyprpilot/commit/95df7d5159a0b148285d37dc9aae9c2340a6df16))
* **adapters:** relocate Tauri commands to the generic layer ([83c3cbb](https://github.com/hyprpilot/hyprpilot/commit/83c3cbbab378e460370913fb73f05e59489477d8))
* **adapters:** typed transcript pipeline + actor lifecycle on AcpInstance ([af6b853](https://github.com/hyprpilot/hyprpilot/commit/af6b8536b806ceed84341252de3628f7df6f4ec5))
* **adapters:** typed transcript pipeline + actor lifecycle on AcpInstance ([2b8b939](https://github.com/hyprpilot/hyprpilot/commit/2b8b939dfb40c55d1d48b4c1c956598b46173737))
* backend refinement batch (12 cleanups) ([e85fab1](https://github.com/hyprpilot/hyprpilot/commit/e85fab1eccec46046372427a7103a2b6750f6675))
* backend refinement batch (12 cleanups) ([e034707](https://github.com/hyprpilot/hyprpilot/commit/e0347079226cc80be13d8e9b04c4e41529a6cffc))
* bullshit-detection audit — 15 of 20 opportunities ([90d65d0](https://github.com/hyprpilot/hyprpilot/commit/90d65d0edae455f751d03823ab1359aafab6ca11))
* bullshit-detection audit — 15 of 20 opportunities ([578de66](https://github.com/hyprpilot/hyprpilot/commit/578de6660101154d64d88ee1fbc08918f71edcdc))
* clean the handlers for commands ([2051781](https://github.com/hyprpilot/hyprpilot/commit/205178105f3c3ee6df1c61e96fe9aa9cb17c64ae))
* cleanup round 2 — permission transparency, wire-title honesty, shared chrome ([dee636d](https://github.com/hyprpilot/hyprpilot/commit/dee636d9138778488b92af42390b18c32e869375))
* cleanup round 2 — permission transparency, wire-title honesty, shared chrome ([cad89d8](https://github.com/hyprpilot/hyprpilot/commit/cad89d8f661df8a3db753881afd58048a73be5fb))
* **config:** adopt merge crate, fold validators into garde, split mod.rs ([f99fcaa](https://github.com/hyprpilot/hyprpilot/commit/f99fcaa0a5bdc3f5385ceb985eda3d562003847e))
* **config:** adopt merge crate, fold validators into garde, split mod.rs ([8557370](https://github.com/hyprpilot/hyprpilot/commit/85573700b111eac3be99dc8c8f1340d00117cd4a))
* **config:** introduce HexColor newtype for theme colour fields ([60cb2bb](https://github.com/hyprpilot/hyprpilot/commit/60cb2bb773f37c78a1fc3bad0b8a0f64da01702a))
* **config:** Logging.level uses logging::LogLevel enum directly ([031c7c8](https://github.com/hyprpilot/hyprpilot/commit/031c7c878d2c38036b28582a87b1db83bea93c7a))
* **config:** make defaults.toml the single source of default values ([b141c6d](https://github.com/hyprpilot/hyprpilot/commit/b141c6d6ba66673cf96cbcc84b34618ab914a84b))
* **config:** make defaults.toml the single source of default values ([bfa6804](https://github.com/hyprpilot/hyprpilot/commit/bfa68042a0a2ae8dc9a90a000450151041afa41f))
* **config:** move active_agent under [agent] section ([0aacb27](https://github.com/hyprpilot/hyprpilot/commit/0aacb27aabc52b637498ab2b5dccadba7fe4b6a7))
* **config:** move validators to config/validations.rs ([b04e63f](https://github.com/hyprpilot/hyprpilot/commit/b04e63f86b5da34db83b3426dfde752a1f0d72ec))
* **config:** unify layer merging behind a Merge trait ([f524c3d](https://github.com/hyprpilot/hyprpilot/commit/f524c3d370dbf98c68a93be838baf1034920f8d6))
* **core:** adapters/ scaffold — Adapter trait + generic types ([ac8d10a](https://github.com/hyprpilot/hyprpilot/commit/ac8d10a65592991a386a98d73243a15639ae7650))
* **core:** relocate acp/ → adapters/acp/; session→instance renames; Tauri event rename ([104ecfb](https://github.com/hyprpilot/hyprpilot/commit/104ecfb490a17cb2c887f273a5b18358c6087ba7))
* **core:** src/adapters layout — Adapter trait, ACP as an impl, session→instance, Acp prefix audit ([fa0f08e](https://github.com/hyprpilot/hyprpilot/commit/fa0f08ea1e2fa01fb2a991e1e7e0b2cefdf2d51e))
* **ctl:** collapse handler boilerplate into single-match dispatch ([f4cca1f](https://github.com/hyprpilot/hyprpilot/commit/f4cca1f515c0d9ed42194db68d252ee7d1ab98cd))
* **ctl:** collapse handler boilerplate into single-match dispatch ([df288db](https://github.com/hyprpilot/hyprpilot/commit/df288dbb770104b050100c3fa66f718255227ae1))
* **daemon:** split run() + extract desktop integration ([6556b7f](https://github.com/hyprpilot/hyprpilot/commit/6556b7f99d99ef6287e8ba376b9d8d52b5d54e64))
* **daemon:** split run() + extract desktop integration ([b367b71](https://github.com/hyprpilot/hyprpilot/commit/b367b718c5e04909f6fe619409cc7fd5e78e1b7e))
* **mcp:** strip speculative broadcast; relocate MCPDefinition ([9f0a196](https://github.com/hyprpilot/hyprpilot/commit/9f0a1964555c66a4262989795a67a5b655cea609))
* **mcp:** strip speculative broadcast; relocate MCPDefinition ([06ee206](https://github.com/hyprpilot/hyprpilot/commit/06ee2063b803f77c7e2471fd05d5a5ead62788a6))
* **rpc:** prune surface, make trait load-bearing, write_line helper ([b68e408](https://github.com/hyprpilot/hyprpilot/commit/b68e40816ace27b70e2c731edfd806721fd86e94))
* **rpc:** prune surface, make trait load-bearing, write_line helper ([2ae0b0a](https://github.com/hyprpilot/hyprpilot/commit/2ae0b0a7d2dfc897d61cb83b3d3680e3e2a5902c))
* **rpc:** signal daemon shutdown via response payload, not a side-channel flag ([b1d88d7](https://github.com/hyprpilot/hyprpilot/commit/b1d88d74ec8bb5356b74666f5b62c6f2a3af5401))
* **rpc:** split CoreHandler into namespaced session/window/daemon handlers ([2c1289e](https://github.com/hyprpilot/hyprpilot/commit/2c1289e1c4a5fa2e56d21b35318cbba20a9459d0))
* skills/paths/logging cleanup — strip dead broadcast, cache BaseDirs, drop home ([d0fc77c](https://github.com/hyprpilot/hyprpilot/commit/d0fc77c53c8f93d61d2addebf29e6302be03254d))
* skills/paths/logging cleanup — strip dead broadcast, cache BaseDirs, drop home ([0e4784b](https://github.com/hyprpilot/hyprpilot/commit/0e4784bd7a5df1f0b5ba2adc62ffdde5485612ed))
* **ui-tools:** unified ToolCallView formatter + per-tool folders ([7b55674](https://github.com/hyprpilot/hyprpilot/commit/7b55674e5bd7cefcd69908449d3162e48b4b4022))
* **ui-tools:** unified ToolCallView formatter + per-tool folders ([0b6a255](https://github.com/hyprpilot/hyprpilot/commit/0b6a2555ec47a88a4093c80b38940f41dd434534))
* **ui:** D5 reskin foundation — theme, shadcn install, audit fixes ([d7b6dfd](https://github.com/hyprpilot/hyprpilot/commit/d7b6dfd06558e7e298c0947936246da181f455ba))
* **ui:** D5 reskin foundation — theme, shadcn install, audit fixes ([eeeabcb](https://github.com/hyprpilot/hyprpilot/commit/eeeabcb483cf86eb753b991f2bbf0a3175eef545))


### Documentation

* **claude-md:** document the ACP scaffold + agents config shape ([97d3f1c](https://github.com/hyprpilot/hyprpilot/commit/97d3f1ca210db69dfcbfb153b43aa8e7a0be6637))
* **claude-md:** document WindowManager adapter + client-side handler pattern ([895b272](https://github.com/hyprpilot/hyprpilot/commit/895b272c3fc23e7211651b74d4f75e05738e7812))
* **claude:** add upstream migration runway + manual verification patterns ([ee93c50](https://github.com/hyprpilot/hyprpilot/commit/ee93c50847f3c44b0909e4a213340ff6a1fbe2ff))
* **claude:** add upstream migration runway + manual verification patterns ([9782401](https://github.com/hyprpilot/hyprpilot/commit/97824012f285db9304e2ece7cf81f6504bae38ad))
* **claude:** codify composition rules from !21 review ([00ec2cb](https://github.com/hyprpilot/hyprpilot/commit/00ec2cb03d03e496ae7122b4e817b60efa74415f))
* **claude:** codify composition rules from !21 review ([4cccfb7](https://github.com/hyprpilot/hyprpilot/commit/4cccfb7e11255882cc495feb500f379ecb67d716))
* **claude:** document session_list + session_load Tauri commands ([a2b2b42](https://github.com/hyprpilot/hyprpilot/commit/a2b2b42378bbc85788e2ebb235986c62b67a50ff))
* **claude:** document session_list + session_load Tauri commands ([645f00d](https://github.com/hyprpilot/hyprpilot/commit/645f00d30b26d36c3032eccd263823e4a877cf90))
* trim oversized comments + capture deviations in CLAUDE.md ([ef01a1c](https://github.com/hyprpilot/hyprpilot/commit/ef01a1c528cca985fd2e976ef1b00a3e38266cc9))

## [0.1.1](https://github.com/hyprpilot/hyprpilot/compare/v0.1.0...v0.1.1) (2026-05-06)


### Features

* **acp+config+palette:** inline-tasks batch — out-of-turn turns, system prompt as attachment, root system_prompt, instance &gt; new ([0ec6e6d](https://github.com/hyprpilot/hyprpilot/commit/0ec6e6df19dfe7d0e3249b14c371c321d3af0057))
* **acp+config+palette:** inline-tasks batch — out-of-turn turns, system prompt as attachment, root system_prompt, instance &gt; new ([1c049c9](https://github.com/hyprpilot/hyprpilot/commit/1c049c93039f7be7cd0537a67a1c505b5f757498))
* **acp:** AcpPermissions — profile allowlists + per-request allow/deny ([afc957c](https://github.com/hyprpilot/hyprpilot/commit/afc957c738036f1a90b55bd57ca5b949f6a61653))
* **acp:** AcpPermissions — profile allowlists + per-request allow/deny ([3badc30](https://github.com/hyprpilot/hyprpilot/commit/3badc30e554d5f4cd63cee51190a4dafdf836b99))
* **acp:** advertise fs+terminal capabilities and implement handlers ([926b056](https://github.com/hyprpilot/hyprpilot/commit/926b05649e68d8bb97ec63185b6f204be028e753))
* **acp:** advertise fs+terminal capabilities and implement handlers ([16da715](https://github.com/hyprpilot/hyprpilot/commit/16da7155cae64eacf43da9164f95b4cc1aa518e2))
* **acp:** bridge daemon JSON-RPC to a coding agent via ACP (scaffold) ([9027b54](https://github.com/hyprpilot/hyprpilot/commit/9027b54a412c6babfc6aa9f2dfa848e5af30b3f9))
* **acp:** live session runtime — spawn, driver, Tauri commands + events ([0cfc3b9](https://github.com/hyprpilot/hyprpilot/commit/0cfc3b98b5ed642ad1a8f1cfaf1228269a276d49))
* **acp:** live session runtime — spawn, driver, Tauri commands + events ([472eaa5](https://github.com/hyprpilot/hyprpilot/commit/472eaa5a7d6c5a4636db0d56810bc015021daf50))
* **acp:** scaffold ACP bridge module + permission fallback chain resolver ([79b1c49](https://github.com/hyprpilot/hyprpilot/commit/79b1c4936889b0d55ac840b4d9da161f8aea2367))
* **acp:** tee subprocess stdout through tracing for wire-level debug ([3190187](https://github.com/hyprpilot/hyprpilot/commit/31901875a696018cedae2defd424bbbb0949017b))
* **acp:** UUID instance keys; multiple of same profile supported ([540d34f](https://github.com/hyprpilot/hyprpilot/commit/540d34f413da77f1947cdd29f5d24b68074b0d3f))
* **acp:** wire session/list + session/load through ACP native RPCs ([cb1cb73](https://github.com/hyprpilot/hyprpilot/commit/cb1cb73625ecf5b80ed41334c0ebe3c0ae1415e3))
* **acp:** wire session/list + session/load through ACP native RPCs ([4b36cda](https://github.com/hyprpilot/hyprpilot/commit/4b36cda66b068c97aa5b4d8cbddfa6e45216cc6d))
* backend-driven presentation (paths + ranker + tool formatters) ([3ce9bff](https://github.com/hyprpilot/hyprpilot/commit/3ce9bff0c3069b0a5827aa69fe38b4e35da09f3d))
* backend-driven presentation (paths + ranker + tool formatters) ([33a49de](https://github.com/hyprpilot/hyprpilot/commit/33a49defc768e9516bc55853fb36408c29bc0adf))
* **composer:** caret-anchored autocomplete with daemon-side sources ([825b04b](https://github.com/hyprpilot/hyprpilot/commit/825b04b724d749fa5c1d029c4bd984e4c952f6ab))
* **composer:** caret-anchored autocomplete with daemon-side sources ([5ca0087](https://github.com/hyprpilot/hyprpilot/commit/5ca008756b8e006f6566cce7f83b26c6e7837285))
* **config:** [[profiles]] with per-profile system_prompt + model overrides ([f8d2820](https://github.com/hyprpilot/hyprpilot/commit/f8d2820de0205275e67d9125ee9b19ea5abbfbfa))
* **config:** [[profiles]] with per-profile system_prompt + model overrides ([312002f](https://github.com/hyprpilot/hyprpilot/commit/312002f7133392ea7d394fb66f064aaefd2cc927))
* **config:** [keymaps] config tree — typed Rust source, collision-validated ([3b9d92b](https://github.com/hyprpilot/hyprpilot/commit/3b9d92bb766abc912cd62a29f8814d9b8cdbf2b1))
* **config:** [keymaps] config tree — typed Rust source, collision-validated ([b096189](https://github.com/hyprpilot/hyprpilot/commit/b096189720f5697320944ec6c8865cdfbee43a05))
* **config:** per-agent model field with per-vendor translation ([095a60f](https://github.com/hyprpilot/hyprpilot/commit/095a60f7350d9d8c455064e39dda212503bfa788))
* **config:** per-agent model field with per-vendor translation ([621a042](https://github.com/hyprpilot/hyprpilot/commit/621a0424a7dcce93c007f7263179dd4f65aa35e6))
* **config:** per-profile mcps / skills / mode / cwd / env / system_prompt ([e5c19f2](https://github.com/hyprpilot/hyprpilot/commit/e5c19f2b561a25f87be251c65690ef62e73ef370))
* **config:** per-profile mcps / skills / mode / cwd / env / system_prompt ([501ffb7](https://github.com/hyprpilot/hyprpilot/commit/501ffb7863390c91b3cd16ed6c817c2ae71f4738))
* **config:** seed agents registry + AcpPermissionPolicy into config layering ([220803c](https://github.com/hyprpilot/hyprpilot/commit/220803c6f6abc9e3f307c95bbd36d55fdffa9381))
* **core:** ctl parity — sessions forget + session-info ([7a57939](https://github.com/hyprpilot/hyprpilot/commit/7a57939ec6c7908af6799f1aec41f45d96942c2e))
* **core:** MCP catalog + socket RPC — global catalog, per-profile enabled set ([043ea47](https://github.com/hyprpilot/hyprpilot/commit/043ea47146b808cac757e00d17245af14b6f7e96))
* **core:** MCP catalog + socket RPC — global catalog, per-profile enabled set ([b1fea74](https://github.com/hyprpilot/hyprpilot/commit/b1fea742efe96093fdb96842b71d705f9cbe0032))
* **core:** sessions/* RPC namespace — list / info / forget ([93e4035](https://github.com/hyprpilot/hyprpilot/commit/93e40351c7d201a2acc90734f8b8edb0afa31756))
* **core:** skills loader + socket RPC + #{skill/name} expansion ([bd9c909](https://github.com/hyprpilot/hyprpilot/commit/bd9c9091b755d3f9bda4f8dc5ebecf866ece3278))
* **core:** skills loader + socket RPC + #{skill/name} expansion ([414686a](https://github.com/hyprpilot/hyprpilot/commit/414686adf2b04aaf3c19baef83bd37bae601349a))
* **core:** socket daemon + diag endpoints — status/version/reload/shutdown + snapshot ([f397393](https://github.com/hyprpilot/hyprpilot/commit/f397393e2a168431ec1ea4c532b1a76a03c7e3d9))
* **core:** socket daemon + diag endpoints — status/version/reload/shutdown + snapshot ([d7b5ec2](https://github.com/hyprpilot/hyprpilot/commit/d7b5ec20dab23e4acdbbfb9c53dcb69c1b959a2b))
* **core:** socket event subscription — events/subscribe + fanout + scoped topics ([3a0fa25](https://github.com/hyprpilot/hyprpilot/commit/3a0fa2565780b8575d5ee2e748569c28573ce9c9))
* **core:** socket event subscription — events/subscribe + fanout + scoped topics ([3ea6189](https://github.com/hyprpilot/hyprpilot/commit/3ea6189f688b818f26dfe161edd783e9bdc133f2))
* **core:** socket overlay control — present/hide/toggle (for hyprland binding) ([6cfb112](https://github.com/hyprpilot/hyprpilot/commit/6cfb112d494c373e9cd26d571a0720dae83d6768))
* **core:** socket overlay control — present/hide/toggle (for hyprland binding) ([fa7e8d0](https://github.com/hyprpilot/hyprpilot/commit/fa7e8d034d8ead9ebd316c5bfebd533a7fdee45d))
* **core:** socket passthroughs — profiles/agents/commands + modes/models per-instance ([7321afb](https://github.com/hyprpilot/hyprpilot/commit/7321afba84bf54d85aec0353e29a29888a2ecb17))
* **core:** socket passthroughs — profiles/agents/commands + modes/models per-instance ([cf54fc5](https://github.com/hyprpilot/hyprpilot/commit/cf54fc504c8d807c6c424b75696e7f373e491f86))
* **core:** socket prompts + permissions — send/cancel + pending/respond ([455cb2c](https://github.com/hyprpilot/hyprpilot/commit/455cb2c722bdfb8a68d9988e0efd87a907fec76e))
* **core:** socket prompts + permissions — send/cancel + pending/respond ([d0d8c92](https://github.com/hyprpilot/hyprpilot/commit/d0d8c92681c9ca53410e6c7b48f2ded350fc0ba5))
* **ctl:** active-instance fallback, rename, overlay/show, auto-spawn ([4d828d4](https://github.com/hyprpilot/hyprpilot/commit/4d828d402f5be5cda0656c83efab4f9a9b62fd52))
* **ctl:** active-instance fallback, rename, overlay/show, auto-spawn ([0023172](https://github.com/hyprpilot/hyprpilot/commit/0023172ece95cfd6212d35021734a2792b6fca30))
* **ctl:** prompts send --draft stages prompt into composer ([2a1a862](https://github.com/hyprpilot/hyprpilot/commit/2a1a8621d9996e802c390e349648e7e521e292e7))
* **ctl:** prompts send --draft stages prompt into composer ([759ba85](https://github.com/hyprpilot/hyprpilot/commit/759ba85e8c6d8c4f3560e5b3c57027ece7ce2203))
* **ctl:** sessions list / info / forget subcommands ([d4da51a](https://github.com/hyprpilot/hyprpilot/commit/d4da51aaf762a8b5fa218e6ce253dd5b13d4346e))
* **ctl:** waybar integration via ctl status JSON stream ([7d5eebf](https://github.com/hyprpilot/hyprpilot/commit/7d5eebf0e7a4b4090bb6ebba10965d8a970bbb61))
* **ctl:** waybar integration via ctl status JSON stream ([1c66149](https://github.com/hyprpilot/hyprpilot/commit/1c661490bd3ed9d12a02cef94cab5d6d83617f12))
* **daemon:** anchor window via zwlr_layer_shell_v1 with center fallback ([ca968e6](https://github.com/hyprpilot/hyprpilot/commit/ca968e6d055cfc7307f45a6f733728e358eacc2b))
* **daemon:** anchor window via zwlr_layer_shell_v1 with center fallback ([6a28916](https://github.com/hyprpilot/hyprpilot/commit/6a28916e61440f22a0986800f0a24a2e8a91f81a))
* **daemon:** autostart plugin + system tray + hidden-by-default boot ([19e94ea](https://github.com/hyprpilot/hyprpilot/commit/19e94ead65460865a8d620a580d88ad7f082c143))
* **daemon:** autostart plugin + system tray + hidden-by-default boot ([da0b06d](https://github.com/hyprpilot/hyprpilot/commit/da0b06db7ad79076d9ffe7287c593317fbb23452))
* **daemon:** default anchor to 40% width, full-height fill ([d37c5b9](https://github.com/hyprpilot/hyprpilot/commit/d37c5b91e4faa72d4393439942cb8249395fdb34))
* **daemon:** full-height overlay default, 40% width, percentage anchor dimensions ([ae8fad0](https://github.com/hyprpilot/hyprpilot/commit/ae8fad00fc1b8c3c341cd7b31180fa879ab1e392))
* **daemon:** wire SIGINT + SIGTERM through clean shutdown orchestrator ([b0c67a3](https://github.com/hyprpilot/hyprpilot/commit/b0c67a3b6207428f69c1407a7a4948c8f5c7fa74))
* **formatter:** tool-call stats — Vec&lt;Stat&gt; wire shape + per-stat mini-pills ([d15dfa3](https://github.com/hyprpilot/hyprpilot/commit/d15dfa36ef1c9b543d760d96fb2c525237f62c9f))
* **formatter:** tool-call stats — Vec&lt;Stat&gt; wire shape + per-stat mini-pills ([57af771](https://github.com/hyprpilot/hyprpilot/commit/57af77104e90e7cfc036267d1ca67896e7843a4b))
* **keymaps+queue+tray:** captain-driven approvals, queue dispatch, tray toggle-only ([3e8ff69](https://github.com/hyprpilot/hyprpilot/commit/3e8ff6988299c32be37ded1d55504205e30a1012))
* **keymaps+queue+tray:** captain-driven approvals, queue dispatch, tray toggle-only ([ae3f29e](https://github.com/hyprpilot/hyprpilot/commit/ae3f29e1ed667de551c96c7419867b6830564296))
* **mcp+permissions:** JSON-file MCP config + unified PermissionController pipeline + MCP tool UI fixes ([abe7e32](https://github.com/hyprpilot/hyprpilot/commit/abe7e32856675ebc8af8790dc4fdd956a9735f0b))
* **mcp+permissions:** JSON-file MCP config + unified PermissionController pipeline + MCP tool UI fixes ([ba96c24](https://github.com/hyprpilot/hyprpilot/commit/ba96c2428240eeecdc67a36f0500b152f2543612))
* **rpc:** explicit shutdown orchestration for daemon/kill ([9f9fa26](https://github.com/hyprpilot/hyprpilot/commit/9f9fa26c0cf275369502bd074ef7d127a190d8b3))
* **rpc:** JSON-RPC 2.0 over the daemon socket + ctl wiring ([0ee500f](https://github.com/hyprpilot/hyprpilot/commit/0ee500f7612fa228cd34e9fad3db82583bd26878))
* **rpc:** JSON-RPC 2.0 over the daemon socket + ctl wiring ([5bb2076](https://github.com/hyprpilot/hyprpilot/commit/5bb2076763f18eef8c785c2afc013aed769b654b))
* **scaffold:** bootstrap Cargo + Tauri 2 + Vue 3 + shadcn-vue repo ([705fe7e](https://github.com/hyprpilot/hyprpilot/commit/705fe7e151bd2cab93590a6b0530e58ec6c8e459))
* **scaffold:** bootstrap Cargo + Tauri 2 + Vue 3 + shadcn-vue repo ([7f7fcc3](https://github.com/hyprpilot/hyprpilot/commit/7f7fcc322d9c330b68b54b9bb03d9a11f7180724))
* **ui:** chat view — transcript, composer, profile switcher, session list ([3235981](https://github.com/hyprpilot/hyprpilot/commit/32359812c80eeae56060233126c4e30e8bff4fe2))
* **ui:** chat view — transcript, composer, profile switcher, session list ([e024b9e](https://github.com/hyprpilot/hyprpilot/commit/e024b9e9eb2dad36ff2836bc973d7963a81e9ddb))
* **ui:** ChatBody renders agent text through markdown pipeline ([9deeded](https://github.com/hyprpilot/hyprpilot/commit/9deeded1bd2d8e0641774e35b027cccb13d51a1b))
* **ui:** command palette primitive — recursive overlay, fuzzy filter, multi/select modes, stub root leaves ([b8a8b3e](https://github.com/hyprpilot/hyprpilot/commit/b8a8b3e587b9d4b717d1c2b8e121a70f8b30ec0d))
* **ui:** command palette primitive — recursive overlay, fuzzy filter, multi/select modes, stub root leaves ([e5550bb](https://github.com/hyprpilot/hyprpilot/commit/e5550bb17716f5facb368c37f51222eb5df2c0a7))
* **ui:** composer state — pills, token expansion, Ctrl+P clipboard image paste ([7a74fe3](https://github.com/hyprpilot/hyprpilot/commit/7a74fe3a476f02b3c102525b0c604fba91ac0c78))
* **ui:** composer state — pills, token expansion, Ctrl+P clipboard image paste ([4e14326](https://github.com/hyprpilot/hyprpilot/commit/4e1432609e5ac7797d0e3de67b58146c0471f625))
* **ui:** design primitives from Claude wireframe bundle — tokens, chrome, chat, command-palette, screen fixtures + Chat.vue migration ([e565fb6](https://github.com/hyprpilot/hyprpilot/commit/e565fb643f4e7b52be59286b57e84e8628e5ff73))
* **ui:** design primitives from Claude wireframe bundle — tokens, chrome, chat, command-palette, screen fixtures + Chat.vue migration ([6cd5ab3](https://github.com/hyprpilot/hyprpilot/commit/6cd5ab34310ce2ff0b090adf5ec5f69ed703bc10))
* **ui:** header wiring — SessionInfoUpdate title + breadcrumbs row ([d96352b](https://github.com/hyprpilot/hyprpilot/commit/d96352b2d0a6d2764b3520b3f3bc4dba9fa5c7ba))
* **ui:** header wiring — SessionInfoUpdate title + breadcrumbs row ([97c64c4](https://github.com/hyprpilot/hyprpilot/commit/97c64c45697e15d0e232872a279d36f4a9fe8b1e))
* **ui:** inline terminal card wiring — terminal/output + terminal/wait_for_exit streaming ([34ad5c0](https://github.com/hyprpilot/hyprpilot/commit/34ad5c0e056477b981cd91fec6a02bc97ba440e4))
* **ui:** inline terminal card wiring — terminal/output + terminal/wait_for_exit streaming ([277228f](https://github.com/hyprpilot/hyprpilot/commit/277228f3bd585e1a447377158dc3fa453ee53158))
* **ui:** markdown + Shiki per-codeblock rendering in agent output ([4b6ef46](https://github.com/hyprpilot/hyprpilot/commit/4b6ef4640f939a36224e16ed17c86de8a3d9f731))
* **ui:** markdown renderer — markdown-it + Shiki + DOMPurify ([bd6fe4c](https://github.com/hyprpilot/hyprpilot/commit/bd6fe4c6e5d281797df64ae0c2a9e83965559c3a))
* **ui:** overlay shell rename — Chat.vue → Overlay.vue ([591356f](https://github.com/hyprpilot/hyprpilot/commit/591356f0170d683ad4ea3ad3fb432a82ff813f39))
* **ui:** overlay shell rename — Chat.vue → Overlay.vue ([1c0eb31](https://github.com/hyprpilot/hyprpilot/commit/1c0eb31fe0e8b4faea0cc70198dbf0cc262cf102))
* **ui:** paint window-edge accent on inward side of the overlay ([d846df4](https://github.com/hyprpilot/hyprpilot/commit/d846df474a34162918742733f1839393e9c3f89d))
* **ui:** paint window-edge accent on the inward side of the overlay ([688d263](https://github.com/hyprpilot/hyprpilot/commit/688d263e7519b11818737208c8adf130adb15e62))
* **ui:** palette leaf — commands (insert slash-name into composer) ([78bcb2f](https://github.com/hyprpilot/hyprpilot/commit/78bcb2feeb14425e931afe12243e894847ac3dd5))
* **ui:** palette leaf — commands (insert slash-name into composer) ([10372ea](https://github.com/hyprpilot/hyprpilot/commit/10372ead3cde080b2abf9883d5650972f4c84f12))
* **ui:** palette leaf — cwd (single-select) with session restart on change ([41fecd5](https://github.com/hyprpilot/hyprpilot/commit/41fecd5e513eff609e8b481690017b5047e68d80))
* **ui:** palette leaf — cwd (single-select) with session restart on change ([fadc806](https://github.com/hyprpilot/hyprpilot/commit/fadc8067e81872370d50daafff5a1e4ca5618b96))
* **ui:** palette leaf — instances (single-select, focus/shutdown) + active-instance store ([48af4c0](https://github.com/hyprpilot/hyprpilot/commit/48af4c0780137e041076209bfdd3827bf37d93db))
* **ui:** palette leaf — instances (single-select, focus/shutdown) + active-instance store ([fd348e3](https://github.com/hyprpilot/hyprpilot/commit/fd348e33ba8b0b634a6d6348b9ec40d2474ef2e9))
* **ui:** palette leaf — MCPs (multi-select) with session restart on change ([2836536](https://github.com/hyprpilot/hyprpilot/commit/28365364098bfe0d7e912dc31c457f89459af215))
* **ui:** palette leaf — MCPs (multi-select) with session restart on change ([9252199](https://github.com/hyprpilot/hyprpilot/commit/9252199d594b3e53bf217f3432951013afaac53e))
* **ui:** palette leaf — profiles (single-select) ([bc7e623](https://github.com/hyprpilot/hyprpilot/commit/bc7e623a58e3ac625ee9c85289106826c555046d))
* **ui:** palette leaf — profiles (single-select) ([96f78a5](https://github.com/hyprpilot/hyprpilot/commit/96f78a5cf89927c3b27f8fa271186c1fdf701be3))
* **ui:** palette leaf — sessions with preview + Ctrl+D delete + session/load ([fd194b1](https://github.com/hyprpilot/hyprpilot/commit/fd194b1c9f930842bc0745764fffdc0e4efa0638))
* **ui:** palette leaf — sessions with preview + Ctrl+D delete + session/load ([7c9914a](https://github.com/hyprpilot/hyprpilot/commit/7c9914a76141ef0451d727aef4ca855a5b80cb29))
* **ui:** palette leaf — skills (multi-select) ([1a4fdf9](https://github.com/hyprpilot/hyprpilot/commit/1a4fdf9421a7b8f2e2d2efb27bb7a29e003508e1))
* **ui:** palette leaf — skills (multi-select) ([9e97578](https://github.com/hyprpilot/hyprpilot/commit/9e97578af1f9b38c063ac0ff9dcd723edaa6df2c))
* **ui:** palette leaves — models + modes (single-select each) ([fd20f67](https://github.com/hyprpilot/hyprpilot/commit/fd20f670a7133924658572ff8ca687980ca45d94))
* **ui:** palette leaves — models + modes (single-select each) ([12667cb](https://github.com/hyprpilot/hyprpilot/commit/12667cbb97b3e8f8cc72a9d496014c04fa1c2322))
* **ui:** permission reply wiring — PermissionStack allow/deny → permission_reply ([03f0fb3](https://github.com/hyprpilot/hyprpilot/commit/03f0fb3a45c4f294f6c00b48b31988d3c6c7b7ae))
* **ui:** permission reply wiring — PermissionStack allow/deny → permission_reply ([e817f62](https://github.com/hyprpilot/hyprpilot/commit/e817f624dadbfe4c7b4722c36f650ca0aeee00a1))
* **ui:** phase state machine — per-instance idle/working/streaming/pending/awaiting ([1b6c4f9](https://github.com/hyprpilot/hyprpilot/commit/1b6c4f9bd9c0df7b153d9d39f45c3c67bf4809d1))
* **ui:** phase state machine — per-instance idle/working/streaming/pending/awaiting ([8149722](https://github.com/hyprpilot/hyprpilot/commit/81497228ea0fa93217424d8fd9244555655caa9a))
* **ui:** queue state machine — FIFO above composer, dispatch on turn_complete ([3260e21](https://github.com/hyprpilot/hyprpilot/commit/3260e21a54157eaebf3b3b1dc8a7ca99555879e8))
* **ui:** queue state machine — FIFO above composer, dispatch on turn_complete ([ba9e115](https://github.com/hyprpilot/hyprpilot/commit/ba9e115d9820c71810aa61f1d4677b6b214b5791))
* **ui:** session event demux — per-primitive typed stores keyed by instance id ([8c202dc](https://github.com/hyprpilot/hyprpilot/commit/8c202dc52c635789979e098617615b3e7b3763fa))
* **ui:** session event demux — per-primitive typed stores keyed by instance id ([eb3812d](https://github.com/hyprpilot/hyprpilot/commit/eb3812dcc0cbb394f173605afb623736e81b4742))
* **ui:** tauri-plugin-log bridge + lib/log.ts + instrument user actions ([336f916](https://github.com/hyprpilot/hyprpilot/commit/336f9161becacfee103fa45cf6a082867921a97a))
* **ui:** tauri-plugin-log bridge + lib/log.ts + instrument user actions ([b1a3ee5](https://github.com/hyprpilot/hyprpilot/commit/b1a3ee5f48dadac65fc7046c818b970dfb36e7bd))
* **ui:** toast row driver — 4s transient status stack ([b7bb947](https://github.com/hyprpilot/hyprpilot/commit/b7bb94725ee93510cd387b34d954f8fb74890e77))
* **ui:** toast row driver — 4s transient status stack ([0c26aa0](https://github.com/hyprpilot/hyprpilot/commit/0c26aa0ce36641ebf0a621581d029f3bdcf204f7))


### Bug Fixes

* **acp:** bump subprocess shutdown grace window 2s → 5s ([3770a41](https://github.com/hyprpilot/hyprpilot/commit/3770a419b5e88c0d8e6e9068a722e7920857f979))
* **acp:** enable unstable feature for newer SessionUpdate variants ([8018af4](https://github.com/hyprpilot/hyprpilot/commit/8018af47475b1d4e04680c14aa277763bf640cac))
* **acp:** enable unstable feature for newer SessionUpdate variants ([66abe5f](https://github.com/hyprpilot/hyprpilot/commit/66abe5f427e74a0811411903e89f9a4c9fb7edfc))
* **acp:** mirror configOptions mode/model flips into RwLocks ([7e1c032](https://github.com/hyprpilot/hyprpilot/commit/7e1c032689921d6aadf4c6b4a883ba20c40557ad))
* **acp:** mirror configOptions mode/model flips into RwLocks ([fb120ea](https://github.com/hyprpilot/hyprpilot/commit/fb120ea66f3f3b580fe5342429b009198d6dad63))
* **acp:** race child.wait() against connect_with for fast dead-child detection ([06827c9](https://github.com/hyprpilot/hyprpilot/commit/06827c9ac9a6324040e17887231dadb538ec9830))
* **acp:** universal synthetic-turn close timer (queue-stuck regression) ([df87d49](https://github.com/hyprpilot/hyprpilot/commit/df87d497827c51320994b1a82a87f29a0eb25244))
* **acp:** universal synthetic-turn close timer (queue-stuck regression) ([7229b65](https://github.com/hyprpilot/hyprpilot/commit/7229b652ab14edd58327667ce397f8484aa392b4))
* **adapters:** address K-251 MR !34 review ([93ae531](https://github.com/hyprpilot/hyprpilot/commit/93ae5313a69dbea9030bc97fadbf8d146d551ace))
* **ci:** hydrate Git LFS on checkout + jsdom-safe scrollIntoView ([f900753](https://github.com/hyprpilot/hyprpilot/commit/f90075313c83ee5401a493fe4802d98b649939d4))
* **ci:** hydrate Git LFS on checkout + jsdom-safe scrollIntoView ([a01b756](https://github.com/hyprpilot/hyprpilot/commit/a01b7569410dfe6b1c2c0c115763382db2eeacc4))
* cleanup a bit ([37cd273](https://github.com/hyprpilot/hyprpilot/commit/37cd273ab54894f9cd5ba3b5a0c367c6f12d1cf2))
* cleanup further ([a37b8b0](https://github.com/hyprpilot/hyprpilot/commit/a37b8b00da127da2cbb7839cfb729e1e45079b64))
* **composer:** keep empty-textarea inline height unset ([cf0c69c](https://github.com/hyprpilot/hyprpilot/commit/cf0c69c39edde4f4c0990c1c28ca1a75818ce241))
* **composer:** keep empty-textarea inline height unset ([33bbf69](https://github.com/hyprpilot/hyprpilot/commit/33bbf69be815ab352ffa64eb1c2681b11040d528))
* **composer:** pipe active-instance cwd into autocomplete query ([4534e94](https://github.com/hyprpilot/hyprpilot/commit/4534e94a3fb50482f7ba923a0c3fd1765786673b))
* **composer:** pipe active-instance cwd into autocomplete query ([a55bd95](https://github.com/hyprpilot/hyprpilot/commit/a55bd95d765350cee6f2ea28d79bc2bb9b3e3b3b))
* **core:** close TerminalStream enum — squash-merge dropped brace ([f4cc8e7](https://github.com/hyprpilot/hyprpilot/commit/f4cc8e785f84d83360cefefd66598c0426a50423))
* **core:** close TerminalStream enum — squash-merge dropped brace ([befc143](https://github.com/hyprpilot/hyprpilot/commit/befc1432ec9a182bae3750b4f92472535be635ca))
* **core:** resolve tracing subscriber double-install at startup ([9af7e8a](https://github.com/hyprpilot/hyprpilot/commit/9af7e8a635a464c3ef4db0ec920226b192e4a0a4))
* **daemon:** resolve anchor width/height against current monitor on every show ([e90d06e](https://github.com/hyprpilot/hyprpilot/commit/e90d06e8b753726e9e4fd504e7dcf0f6ef650845))
* **daemon:** resolve anchor width/height against current monitor on every show ([0e071c6](https://github.com/hyprpilot/hyprpilot/commit/0e071c6bae94081cbcffe0264edc7e25f9846654))
* **deps:** update dependency @vueuse/core to v14 ([1a5a6e8](https://github.com/hyprpilot/hyprpilot/commit/1a5a6e878f28989eafb4437a4d32e197b0ada832))
* **deps:** update dependency @vueuse/core to v14 ([923dd32](https://github.com/hyprpilot/hyprpilot/commit/923dd325dc20d4bfa8a837b89cd6eea6b43c397c))
* **deps:** update dependency shiki to v4 ([8a5eac1](https://github.com/hyprpilot/hyprpilot/commit/8a5eac10be6ea8c16fabc986618a8b90cfa3861b))
* **deps:** update dependency shiki to v4 ([d984c50](https://github.com/hyprpilot/hyprpilot/commit/d984c500d9d0ff7a655c31baf6fea895ea3620aa))
* **mcps+effort:** per-instance MCP catalog + config-option banner ([521920f](https://github.com/hyprpilot/hyprpilot/commit/521920fc39427882a2edd47284370b0459b07daf))
* **mcps+effort:** per-instance MCP catalog + config-option banner ([1b7782f](https://github.com/hyprpilot/hyprpilot/commit/1b7782f26f259bad870212b04a706eff8e428ba2))
* paths_resolve flat args + thread cwd through session_load ([4baeb86](https://github.com/hyprpilot/hyprpilot/commit/4baeb86a24b0e4ccbe14c2bc3a5cf8f1280302ab))
* paths_resolve flat args + thread cwd through session_load ([e1bd571](https://github.com/hyprpilot/hyprpilot/commit/e1bd571bb54c35dfff1d041e99e43e6642880220))
* **permissions:** Ctrl+G / Ctrl+R bind only to basic-once variants ([98dd222](https://github.com/hyprpilot/hyprpilot/commit/98dd222a65860ef792d0d13cc27f65ea6e1ff525))
* **permissions:** Ctrl+G / Ctrl+R bind only to basic-once variants ([5f3bdc8](https://github.com/hyprpilot/hyprpilot/commit/5f3bdc8cea9c7165f67712fe2cfaa2dfbd159afd))
* queue stuck + claude-code thought extraction + user-msg markdown + composer 50vh ([3b76f5e](https://github.com/hyprpilot/hyprpilot/commit/3b76f5eab9c2330cd55594aa2fcd89a43f189688))
* queue stuck + claude-code thought extraction + user-msg markdown + composer 50vh ([45c871f](https://github.com/hyprpilot/hyprpilot/commit/45c871f9f85cb35fd664fe273a2d6437cc4b38b4))
* **sessions:** track focused-instance profile, not picker selection ([c948357](https://github.com/hyprpilot/hyprpilot/commit/c9483578ecba4b9fe156c1a18f07a0634d4425d8))
* subprocess stderr capture, tool-call success color, per-turn grouping ([74fecf0](https://github.com/hyprpilot/hyprpilot/commit/74fecf08da58422701ff04de15f80be046b23033))
* **terminal:** drain pipe readers before wait() returns ([1d3763f](https://github.com/hyprpilot/hyprpilot/commit/1d3763f39023f40e3f867dd75a6cce4d493c87da))
* **ui:** MarkdownBody — collapse the giant gap on GFM task-list rows ([a846672](https://github.com/hyprpilot/hyprpilot/commit/a84667289443089b892a07a046702a9bc9c95631))
* **ui:** plan modal must overlay viewport, not the scrolled chat region ([b8dcede](https://github.com/hyprpilot/hyprpilot/commit/b8dcedeb71ae7faff1221fed2f0888161f7a006a))
* **ui:** push user turn before submit invoke so it wins the seq race ([58c01b1](https://github.com/hyprpilot/hyprpilot/commit/58c01b14d11e3b32401db5f0aab91a377aeb6ac2))
* **ui:** resolve pre-existing vue-tsc --noEmit failures ([5870752](https://github.com/hyprpilot/hyprpilot/commit/58707520d2de80bf768685f394d289495c5f8d3f))
* **ui:** resolve pre-existing vue-tsc --noEmit failures ([9a30bf6](https://github.com/hyprpilot/hyprpilot/commit/9a30bf6eaa7c403a434b751987040f300661a7b9))
* **ui:** tighten modal trigger + diagnostic for missing permission prompts ([d9f8a44](https://github.com/hyprpilot/hyprpilot/commit/d9f8a4464c78fa5b7e1fa551c2cbba25c238b507))
* update arguments ([872acf7](https://github.com/hyprpilot/hyprpilot/commit/872acf7a3f04050e38552dd36a9324d99313aa59))
* update basic configuration ([5403e09](https://github.com/hyprpilot/hyprpilot/commit/5403e09896a78db96bfa928a99ec934ae148ce5e))
* update defaults ([0484144](https://github.com/hyprpilot/hyprpilot/commit/048414429d915005ddbe3b92a3b95f367a2804fc))
* update defaults ([f37bd1f](https://github.com/hyprpilot/hyprpilot/commit/f37bd1f0f99904dc318d625a2ef3afffe4a7cc99))
* update dependency ([71d9852](https://github.com/hyprpilot/hyprpilot/commit/71d9852c98dd6cff35d6874150f01de9bb331400))


### Refactor

* **acp:** drop AcpPermissionPolicy — PermissionController is separate scope ([9b4c567](https://github.com/hyprpilot/hyprpilot/commit/9b4c5679a80d041f7fb0dae4637c6d85e767cde2))
* **adapters:** relocate Tauri commands to the generic layer ([95df7d5](https://github.com/hyprpilot/hyprpilot/commit/95df7d5159a0b148285d37dc9aae9c2340a6df16))
* **adapters:** relocate Tauri commands to the generic layer ([83c3cbb](https://github.com/hyprpilot/hyprpilot/commit/83c3cbbab378e460370913fb73f05e59489477d8))
* **adapters:** typed transcript pipeline + actor lifecycle on AcpInstance ([af6b853](https://github.com/hyprpilot/hyprpilot/commit/af6b8536b806ceed84341252de3628f7df6f4ec5))
* **adapters:** typed transcript pipeline + actor lifecycle on AcpInstance ([2b8b939](https://github.com/hyprpilot/hyprpilot/commit/2b8b939dfb40c55d1d48b4c1c956598b46173737))
* backend refinement batch (12 cleanups) ([e85fab1](https://github.com/hyprpilot/hyprpilot/commit/e85fab1eccec46046372427a7103a2b6750f6675))
* backend refinement batch (12 cleanups) ([e034707](https://github.com/hyprpilot/hyprpilot/commit/e0347079226cc80be13d8e9b04c4e41529a6cffc))
* bullshit-detection audit — 15 of 20 opportunities ([90d65d0](https://github.com/hyprpilot/hyprpilot/commit/90d65d0edae455f751d03823ab1359aafab6ca11))
* bullshit-detection audit — 15 of 20 opportunities ([578de66](https://github.com/hyprpilot/hyprpilot/commit/578de6660101154d64d88ee1fbc08918f71edcdc))
* clean the handlers for commands ([2051781](https://github.com/hyprpilot/hyprpilot/commit/205178105f3c3ee6df1c61e96fe9aa9cb17c64ae))
* cleanup round 2 — permission transparency, wire-title honesty, shared chrome ([dee636d](https://github.com/hyprpilot/hyprpilot/commit/dee636d9138778488b92af42390b18c32e869375))
* cleanup round 2 — permission transparency, wire-title honesty, shared chrome ([cad89d8](https://github.com/hyprpilot/hyprpilot/commit/cad89d8f661df8a3db753881afd58048a73be5fb))
* **config:** adopt merge crate, fold validators into garde, split mod.rs ([f99fcaa](https://github.com/hyprpilot/hyprpilot/commit/f99fcaa0a5bdc3f5385ceb985eda3d562003847e))
* **config:** adopt merge crate, fold validators into garde, split mod.rs ([8557370](https://github.com/hyprpilot/hyprpilot/commit/85573700b111eac3be99dc8c8f1340d00117cd4a))
* **config:** introduce HexColor newtype for theme colour fields ([60cb2bb](https://github.com/hyprpilot/hyprpilot/commit/60cb2bb773f37c78a1fc3bad0b8a0f64da01702a))
* **config:** Logging.level uses logging::LogLevel enum directly ([031c7c8](https://github.com/hyprpilot/hyprpilot/commit/031c7c878d2c38036b28582a87b1db83bea93c7a))
* **config:** make defaults.toml the single source of default values ([b141c6d](https://github.com/hyprpilot/hyprpilot/commit/b141c6d6ba66673cf96cbcc84b34618ab914a84b))
* **config:** make defaults.toml the single source of default values ([bfa6804](https://github.com/hyprpilot/hyprpilot/commit/bfa68042a0a2ae8dc9a90a000450151041afa41f))
* **config:** move active_agent under [agent] section ([0aacb27](https://github.com/hyprpilot/hyprpilot/commit/0aacb27aabc52b637498ab2b5dccadba7fe4b6a7))
* **config:** move validators to config/validations.rs ([b04e63f](https://github.com/hyprpilot/hyprpilot/commit/b04e63f86b5da34db83b3426dfde752a1f0d72ec))
* **config:** unify layer merging behind a Merge trait ([f524c3d](https://github.com/hyprpilot/hyprpilot/commit/f524c3d370dbf98c68a93be838baf1034920f8d6))
* **core:** adapters/ scaffold — Adapter trait + generic types ([ac8d10a](https://github.com/hyprpilot/hyprpilot/commit/ac8d10a65592991a386a98d73243a15639ae7650))
* **core:** relocate acp/ → adapters/acp/; session→instance renames; Tauri event rename ([104ecfb](https://github.com/hyprpilot/hyprpilot/commit/104ecfb490a17cb2c887f273a5b18358c6087ba7))
* **core:** src/adapters layout — Adapter trait, ACP as an impl, session→instance, Acp prefix audit ([fa0f08e](https://github.com/hyprpilot/hyprpilot/commit/fa0f08ea1e2fa01fb2a991e1e7e0b2cefdf2d51e))
* **ctl:** collapse handler boilerplate into single-match dispatch ([f4cca1f](https://github.com/hyprpilot/hyprpilot/commit/f4cca1f515c0d9ed42194db68d252ee7d1ab98cd))
* **ctl:** collapse handler boilerplate into single-match dispatch ([df288db](https://github.com/hyprpilot/hyprpilot/commit/df288dbb770104b050100c3fa66f718255227ae1))
* **daemon:** split run() + extract desktop integration ([6556b7f](https://github.com/hyprpilot/hyprpilot/commit/6556b7f99d99ef6287e8ba376b9d8d52b5d54e64))
* **daemon:** split run() + extract desktop integration ([b367b71](https://github.com/hyprpilot/hyprpilot/commit/b367b718c5e04909f6fe619409cc7fd5e78e1b7e))
* **mcp:** strip speculative broadcast; relocate MCPDefinition ([9f0a196](https://github.com/hyprpilot/hyprpilot/commit/9f0a1964555c66a4262989795a67a5b655cea609))
* **mcp:** strip speculative broadcast; relocate MCPDefinition ([06ee206](https://github.com/hyprpilot/hyprpilot/commit/06ee2063b803f77c7e2471fd05d5a5ead62788a6))
* **rpc:** prune surface, make trait load-bearing, write_line helper ([b68e408](https://github.com/hyprpilot/hyprpilot/commit/b68e40816ace27b70e2c731edfd806721fd86e94))
* **rpc:** prune surface, make trait load-bearing, write_line helper ([2ae0b0a](https://github.com/hyprpilot/hyprpilot/commit/2ae0b0a7d2dfc897d61cb83b3d3680e3e2a5902c))
* **rpc:** signal daemon shutdown via response payload, not a side-channel flag ([b1d88d7](https://github.com/hyprpilot/hyprpilot/commit/b1d88d74ec8bb5356b74666f5b62c6f2a3af5401))
* **rpc:** split CoreHandler into namespaced session/window/daemon handlers ([2c1289e](https://github.com/hyprpilot/hyprpilot/commit/2c1289e1c4a5fa2e56d21b35318cbba20a9459d0))
* skills/paths/logging cleanup — strip dead broadcast, cache BaseDirs, drop home ([d0fc77c](https://github.com/hyprpilot/hyprpilot/commit/d0fc77c53c8f93d61d2addebf29e6302be03254d))
* skills/paths/logging cleanup — strip dead broadcast, cache BaseDirs, drop home ([0e4784b](https://github.com/hyprpilot/hyprpilot/commit/0e4784bd7a5df1f0b5ba2adc62ffdde5485612ed))
* **ui-tools:** unified ToolCallView formatter + per-tool folders ([7b55674](https://github.com/hyprpilot/hyprpilot/commit/7b55674e5bd7cefcd69908449d3162e48b4b4022))
* **ui-tools:** unified ToolCallView formatter + per-tool folders ([0b6a255](https://github.com/hyprpilot/hyprpilot/commit/0b6a2555ec47a88a4093c80b38940f41dd434534))
* **ui:** D5 reskin foundation — theme, shadcn install, audit fixes ([d7b6dfd](https://github.com/hyprpilot/hyprpilot/commit/d7b6dfd06558e7e298c0947936246da181f455ba))
* **ui:** D5 reskin foundation — theme, shadcn install, audit fixes ([eeeabcb](https://github.com/hyprpilot/hyprpilot/commit/eeeabcb483cf86eb753b991f2bbf0a3175eef545))


### Documentation

* **claude-md:** document the ACP scaffold + agents config shape ([97d3f1c](https://github.com/hyprpilot/hyprpilot/commit/97d3f1ca210db69dfcbfb153b43aa8e7a0be6637))
* **claude-md:** document WindowManager adapter + client-side handler pattern ([895b272](https://github.com/hyprpilot/hyprpilot/commit/895b272c3fc23e7211651b74d4f75e05738e7812))
* **claude:** add upstream migration runway + manual verification patterns ([ee93c50](https://github.com/hyprpilot/hyprpilot/commit/ee93c50847f3c44b0909e4a213340ff6a1fbe2ff))
* **claude:** add upstream migration runway + manual verification patterns ([9782401](https://github.com/hyprpilot/hyprpilot/commit/97824012f285db9304e2ece7cf81f6504bae38ad))
* **claude:** codify composition rules from !21 review ([00ec2cb](https://github.com/hyprpilot/hyprpilot/commit/00ec2cb03d03e496ae7122b4e817b60efa74415f))
* **claude:** codify composition rules from !21 review ([4cccfb7](https://github.com/hyprpilot/hyprpilot/commit/4cccfb7e11255882cc495feb500f379ecb67d716))
* **claude:** document session_list + session_load Tauri commands ([a2b2b42](https://github.com/hyprpilot/hyprpilot/commit/a2b2b42378bbc85788e2ebb235986c62b67a50ff))
* **claude:** document session_list + session_load Tauri commands ([645f00d](https://github.com/hyprpilot/hyprpilot/commit/645f00d30b26d36c3032eccd263823e4a877cf90))
* trim oversized comments + capture deviations in CLAUDE.md ([ef01a1c](https://github.com/hyprpilot/hyprpilot/commit/ef01a1c528cca985fd2e976ef1b00a3e38266cc9))
