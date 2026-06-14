## [1.53.0](https://github.com/perice-pope/ai-music-companion/compare/v1.52.0...v1.53.0) (2026-06-14)

### Features

* **score:** quantize rhythm before notation ([#206](https://github.com/perice-pope/ai-music-companion/issues/206)) ([d6247e9](https://github.com/perice-pope/ai-music-companion/commit/d6247e9a3eb1db08212278e85e49f2dd18d6f724))

## [1.52.0](https://github.com/perice-pope/ai-music-companion/compare/v1.51.0...v1.52.0) (2026-06-13)

### Features

* **store:** user_version migrations + session debug columns ([#203](https://github.com/perice-pope/ai-music-companion/issues/203)) ([237e67a](https://github.com/perice-pope/ai-music-companion/commit/237e67a1933804d7471167659dafc0e8c2fee99c))

## [1.51.0](https://github.com/perice-pope/ai-music-companion/compare/v1.50.2...v1.51.0) (2026-06-13)

### Features

* **desktop:** persist per-phrase metrics to a session_phrases table ([#202](https://github.com/perice-pope/ai-music-companion/issues/202)) ([e385015](https://github.com/perice-pope/ai-music-companion/commit/e385015428b54963301848b4950e6334ff5acde3)), closes [#196](https://github.com/perice-pope/ai-music-companion/issues/196)

## [1.50.2](https://github.com/perice-pope/ai-music-companion/compare/v1.50.1...v1.50.2) (2026-06-13)

### Bug Fixes

* **score:** emit <chord/> for simultaneous notes in the MusicXML emitter ([#200](https://github.com/perice-pope/ai-music-companion/issues/200)) ([d2dd027](https://github.com/perice-pope/ai-music-companion/commit/d2dd0277d65e81995a45b50cbd0f7f71b295629f))

## [1.50.1](https://github.com/perice-pope/ai-music-companion/compare/v1.50.0...v1.50.1) (2026-06-13)

### Bug Fixes

* **desktop:** persist completed practice sessions to the on-disk store ([#196](https://github.com/perice-pope/ai-music-companion/issues/196)) ([a1b5ee4](https://github.com/perice-pope/ai-music-companion/commit/a1b5ee42b26861491883324c0d61893c3bec2f5f))
* **tuner:** smooth displayed cents for readability ([#187](https://github.com/perice-pope/ai-music-companion/issues/187)) ([#198](https://github.com/perice-pope/ai-music-companion/issues/198)) ([031b29d](https://github.com/perice-pope/ai-music-companion/commit/031b29db508ee0f6f20230922030024cbaccc332))

## [1.50.0](https://github.com/perice-pope/ai-music-companion/compare/v1.49.0...v1.50.0) (2026-06-13)

### Features

* **desktop:** surface degraded storage to the musician ([#137](https://github.com/perice-pope/ai-music-companion/issues/137)) + salvage Score Mode e2e test ([#183](https://github.com/perice-pope/ai-music-companion/issues/183)) ([#194](https://github.com/perice-pope/ai-music-companion/issues/194)) ([48fdf25](https://github.com/perice-pope/ai-music-companion/commit/48fdf250b679e6228649c714233c458b2f739f3b)), closes [#191](https://github.com/perice-pope/ai-music-companion/issues/191) [#188](https://github.com/perice-pope/ai-music-companion/issues/188) [185/#191](https://github.com/185/ai-music-companion/issues/191) [#185](https://github.com/perice-pope/ai-music-companion/issues/185)

## [1.49.0](https://github.com/perice-pope/ai-music-companion/compare/v1.48.2...v1.49.0) (2026-06-13)

### Features

* **ears:** SuperFlux spectral-flux onset detection, vibrato-robust ([#138](https://github.com/perice-pope/ai-music-companion/issues/138)) ([57eeb68](https://github.com/perice-pope/ai-music-companion/commit/57eeb6867e47ab81dc776556443df3bfe7c90089))

## [1.48.2](https://github.com/perice-pope/ai-music-companion/compare/v1.48.1...v1.48.2) (2026-06-13)

### Bug Fixes

* **startup:** degrade to in-memory with a warning instead of panicking ([#137](https://github.com/perice-pope/ai-music-companion/issues/137)) ([#191](https://github.com/perice-pope/ai-music-companion/issues/191)) ([20900f9](https://github.com/perice-pope/ai-music-companion/commit/20900f9d95d6062d2aa00958c6e6c4d54b8e4b30))

## [1.48.1](https://github.com/perice-pope/ai-music-companion/compare/v1.48.0...v1.48.1) (2026-06-13)

### Bug Fixes

* **session:** record worker-detected phrases into the recap ([#185](https://github.com/perice-pope/ai-music-companion/issues/185) root cause) ([#190](https://github.com/perice-pope/ai-music-companion/issues/190)) ([d93c1c2](https://github.com/perice-pope/ai-music-companion/commit/d93c1c28c564307a5106222af01e31ec02c2e9ba))

## [1.48.0](https://github.com/perice-pope/ai-music-companion/compare/v1.47.1...v1.48.0) (2026-06-12)

### Features

* **import:** PDF → sheet music via on-device OMR (Phase 1, behind beta flag) ([#181](https://github.com/perice-pope/ai-music-companion/issues/181)) ([9774b71](https://github.com/perice-pope/ai-music-companion/commit/9774b7135b84344aa51928f39d01b580cec3fd31))

## [1.47.1](https://github.com/perice-pope/ai-music-companion/compare/v1.47.0...v1.47.1) (2026-06-12)

### Bug Fixes

* **score:** enforce silence > lies — rhythmic_stability is Option<f64>, not fake 1.0 ([#180](https://github.com/perice-pope/ai-music-companion/issues/180)) ([47a6c03](https://github.com/perice-pope/ai-music-companion/commit/47a6c03f716f188aad912904b5917439eacc964a)), closes [#135](https://github.com/perice-pope/ai-music-companion/issues/135)

## [1.47.0](https://github.com/perice-pope/ai-music-companion/compare/v1.46.1...v1.47.0) (2026-06-11)

### Features

* **score:** wire MusicXML import with part selection ([#179](https://github.com/perice-pope/ai-music-companion/issues/179)) ([80ee3ef](https://github.com/perice-pope/ai-music-companion/commit/80ee3ef84a428598198c740c179f5503fae5e2a2))

## [1.46.1](https://github.com/perice-pope/ai-music-companion/compare/v1.46.0...v1.46.1) (2026-06-11)

### Bug Fixes

* **ci:** enforce frontend lint, test, and build gates ([#175](https://github.com/perice-pope/ai-music-companion/issues/175)) ([f0789a2](https://github.com/perice-pope/ai-music-companion/commit/f0789a25810827fab4fefe6445a2683c42c263d3))

## [1.46.0](https://github.com/perice-pope/ai-music-companion/compare/v1.45.0...v1.46.0) (2026-06-11)

### Features

* wire offline idiom engine into practice→recap flow ([#168](https://github.com/perice-pope/ai-music-companion/issues/168)) ([8b73144](https://github.com/perice-pope/ai-music-companion/commit/8b73144bbd518d13692e36214ee439aa06f96f32))

## [1.45.0](https://github.com/perice-pope/ai-music-companion/compare/v1.44.0...v1.45.0) (2026-06-10)

### Features

* offline-first & network-transparency principle + Connections & Privacy surface ([#167](https://github.com/perice-pope/ai-music-companion/issues/167)) ([8d7b407](https://github.com/perice-pope/ai-music-companion/commit/8d7b407f461db3a7744a526aa8ae4302566ad50a))

## [1.44.0](https://github.com/perice-pope/ai-music-companion/compare/v1.43.0...v1.44.0) (2026-06-10)

### Features

* **brain:** cross-genre contextual coaching (Phase 4 LLM-grounded relevance) ([#166](https://github.com/perice-pope/ai-music-companion/issues/166)) ([d842ab0](https://github.com/perice-pope/ai-music-companion/commit/d842ab0113c5b227706872bc298392f19d1ee980))

## [1.43.0](https://github.com/perice-pope/ai-music-companion/compare/v1.42.0...v1.43.0) (2026-06-10)

### Features

* personalization data foundation — fingerprint persistence + taste profile ([#165](https://github.com/perice-pope/ai-music-companion/issues/165)) ([bb2dcc3](https://github.com/perice-pope/ai-music-companion/commit/bb2dcc388f27379708f498a66685f01deba046b6))

## [1.42.0](https://github.com/perice-pope/ai-music-companion/compare/v1.41.1...v1.42.0) (2026-06-10)

### Features

* **idiom:** offline audio-embedding idiom-recognition engine ([#164](https://github.com/perice-pope/ai-music-companion/issues/164)) ([8596d07](https://github.com/perice-pope/ai-music-companion/commit/8596d077219b5dee4ee77df50d275852d8468985))

## [1.41.1](https://github.com/perice-pope/ai-music-companion/compare/v1.41.0...v1.41.1) (2026-06-07)

### Bug Fixes

* **desktop:** add ESLint flat config so `pnpm lint` runs ([#158](https://github.com/perice-pope/ai-music-companion/issues/158)) ([d9a0b94](https://github.com/perice-pope/ai-music-companion/commit/d9a0b94ed6a35c736d5248954e9a79fb82bc55d9))

## [1.41.0](https://github.com/perice-pope/ai-music-companion/compare/v1.40.0...v1.41.0) (2026-06-07)

### Features

* **brain:** wire intonation & groove into the session recap (Phase 4) ([#157](https://github.com/perice-pope/ai-music-companion/issues/157)) ([7b78501](https://github.com/perice-pope/ai-music-companion/commit/7b78501d214ab3907d0d5645ce857e9f3dc1d05d))

## [1.40.0](https://github.com/perice-pope/ai-music-companion/compare/v1.39.1...v1.40.0) (2026-06-07)

### Features

* **groove:** Phase 4 A2 — rhythm & groove analysis crate (tempo, swing, timing) ([#155](https://github.com/perice-pope/ai-music-companion/issues/155)) ([9e75888](https://github.com/perice-pope/ai-music-companion/commit/9e7588879122ce12a6f730f8af552b281bd6a1a0))
* **theory:** real intonation analysis — cents vs ET + tuning-tendency map (Phase 4 A1) ([#156](https://github.com/perice-pope/ai-music-companion/issues/156)) ([ff14d8d](https://github.com/perice-pope/ai-music-companion/commit/ff14d8d1bedcef2ceb7d778c656a1af8c508a669))

## [1.39.1](https://github.com/perice-pope/ai-music-companion/compare/v1.39.0...v1.39.1) (2026-06-06)

### Bug Fixes

* **desktop:** restore Tauri build — add key/session_key to src-tauri recap literals ([#153](https://github.com/perice-pope/ai-music-companion/issues/153)) ([2187fd1](https://github.com/perice-pope/ai-music-companion/commit/2187fd1cca368e68ec055a659f849fcb21367538))

## [1.39.0](https://github.com/perice-pope/ai-music-companion/compare/v1.38.0...v1.39.0) (2026-06-06)

### Features

* **brain:** wire key/mode detection into phrases + recap (Phase 4 PR 2) ([#152](https://github.com/perice-pope/ai-music-companion/issues/152)) ([097c6ef](https://github.com/perice-pope/ai-music-companion/commit/097c6efece88509949e2361db0dfefbf8add97eb)), closes [#151](https://github.com/perice-pope/ai-music-companion/issues/151)

## [1.38.0](https://github.com/perice-pope/ai-music-companion/compare/v1.37.0...v1.38.0) (2026-06-06)

### Features

* **theory:** key & mode detection crate (Phase 4, Track A PR 1) ([#151](https://github.com/perice-pope/ai-music-companion/issues/151)) ([1066baa](https://github.com/perice-pope/ai-music-companion/commit/1066baabcefdd89c4532ee37ef07348d870359d3))

## [1.37.0](https://github.com/perice-pope/ai-music-companion/compare/v1.36.1...v1.37.0) (2026-06-06)

### Features

* **supabase:** teacher↔student linking + RLS swap (privacy core) ([#149](https://github.com/perice-pope/ai-music-companion/issues/149)) ([d9c1dd0](https://github.com/perice-pope/ai-music-companion/commit/d9c1dd03a99383b933e47765944bb335dee9815a))

## [1.36.1](https://github.com/perice-pope/ai-music-companion/compare/v1.36.0...v1.36.1) (2026-06-01)

### Bug Fixes

* hold single coaching service to restore rate limiting ([#107](https://github.com/perice-pope/ai-music-companion/issues/107)) ([3d6255f](https://github.com/perice-pope/ai-music-companion/commit/3d6255fd1f442fd1f0828704d858844e4b6401bc))

## [1.36.0](https://github.com/perice-pope/ai-music-companion/compare/v1.35.1...v1.36.0) (2026-06-01)

### Features

* **desktop:** optional cloud sync for practice sessions ([#145](https://github.com/perice-pope/ai-music-companion/issues/145)) ([b56717e](https://github.com/perice-pope/ai-music-companion/commit/b56717eb2b933c2a7eecff872490912ab2836ef1)), closes [#144](https://github.com/perice-pope/ai-music-companion/issues/144)

## [1.35.1](https://github.com/perice-pope/ai-music-companion/compare/v1.35.0...v1.35.1) (2026-06-01)

### Bug Fixes

* resolve hotspot bugs [#100](https://github.com/perice-pope/ai-music-companion/issues/100), [#102](https://github.com/perice-pope/ai-music-companion/issues/102), [#101](https://github.com/perice-pope/ai-music-companion/issues/101) ([#146](https://github.com/perice-pope/ai-music-companion/issues/146)) ([9ba0fdd](https://github.com/perice-pope/ai-music-companion/commit/9ba0fddcaf94dfed32946b1dce7a47f2609db89f))

## [1.35.0](https://github.com/perice-pope/ai-music-companion/compare/v1.34.0...v1.35.0) (2026-05-31)

### Features

* **tone:** gentle tone read-out in the session recap (Phase 3, tone PR 6) ([#143](https://github.com/perice-pope/ai-music-companion/issues/143)) ([da6d42d](https://github.com/perice-pope/ai-music-companion/commit/da6d42d5804054ec226bcbab741629343f315078))

## [1.34.0](https://github.com/perice-pope/ai-music-companion/compare/v1.33.0...v1.34.0) (2026-05-31)

### Features

* **tone:** persist session tone aggregate for trends (Phase 3, tone PR 5) ([#141](https://github.com/perice-pope/ai-music-companion/issues/141)) ([62d5264](https://github.com/perice-pope/ai-music-companion/commit/62d52646911daf4b36125a765b2163e26f04b77f))

## [1.33.0](https://github.com/perice-pope/ai-music-companion/compare/v1.32.0...v1.33.0) (2026-05-31)

### Features

* **tone:** surface tone in the coach's recap + live tips (Phase 3, tone PR 4) ([#140](https://github.com/perice-pope/ai-music-companion/issues/140)) ([b474930](https://github.com/perice-pope/ai-music-companion/commit/b4749301dc20e4b3a266318e5d3f927fcb841c47))

## [1.32.0](https://github.com/perice-pope/ai-music-companion/compare/v1.31.0...v1.32.0) (2026-05-31)

### Features

* **tone:** compute and attach tone to live phrases (Phase 3, tone PR 3) ([#133](https://github.com/perice-pope/ai-music-companion/issues/133)) ([3b2be51](https://github.com/perice-pope/ai-music-companion/commit/3b2be51cc73f3930c3ae0b552551d28f4e3a5212))

## [1.31.0](https://github.com/perice-pope/ai-music-companion/compare/v1.30.0...v1.31.0) (2026-05-31)

### Features

* **tone:** heuristic descriptor, room calibration, baseline (Phase 3, tone PR 2) ([#132](https://github.com/perice-pope/ai-music-companion/issues/132)) ([df60a0e](https://github.com/perice-pope/ai-music-companion/commit/df60a0eed64e759aaab8bb9446899c59ac6ef9a1))

## [1.30.0](https://github.com/perice-pope/ai-music-companion/compare/v1.29.0...v1.30.0) (2026-05-31)

### Features

* **tone:** timbre feature extraction crate (Phase 3, tone PR 1) ([#131](https://github.com/perice-pope/ai-music-companion/issues/131)) ([f7555d0](https://github.com/perice-pope/ai-music-companion/commit/f7555d04836f1650c012927f5f9fdaa7f75b472c))

## [1.29.0](https://github.com/perice-pope/ai-music-companion/compare/v1.28.0...v1.29.0) (2026-05-31)

### Features

* **desktop:** resolve bundled ONNX Runtime at startup for audio import ([#127](https://github.com/perice-pope/ai-music-companion/issues/127)) ([4a8fe68](https://github.com/perice-pope/ai-music-companion/commit/4a8fe68f2ae9f3bf7eec63a37f4adfe83e3dcd69)), closes [112/#120](https://github.com/112/ai-music-companion/issues/120)

## [1.28.0](https://github.com/perice-pope/ai-music-companion/compare/v1.27.0...v1.28.0) (2026-05-30)

### Features

* **score:** import audio recordings via basic-pitch transcription ([#126](https://github.com/perice-pope/ai-music-companion/issues/126)) ([ca09688](https://github.com/perice-pope/ai-music-companion/commit/ca096882787a56e432bcbaca529d6224c2488fd0)), closes [#125](https://github.com/perice-pope/ai-music-companion/issues/125)

## [1.27.0](https://github.com/perice-pope/ai-music-companion/compare/v1.26.0...v1.27.0) (2026-05-30)

### Features

* **score:** import MIDI files into the score library ([#123](https://github.com/perice-pope/ai-music-companion/issues/123)) ([3d0ea4b](https://github.com/perice-pope/ai-music-companion/commit/3d0ea4b886135a085502373ea9af5c49a54f9c7f))

## [1.26.0](https://github.com/perice-pope/ai-music-companion/compare/v1.25.0...v1.26.0) (2026-05-30)

### Features

* **transcribe:** audio-to-MIDI transcription core (basic-pitch / ONNX) ([#125](https://github.com/perice-pope/ai-music-companion/issues/125)) ([782a445](https://github.com/perice-pope/ai-music-companion/commit/782a445b308189b8f47c5e1be0da7cbce110e228))

## [1.25.0](https://github.com/perice-pope/ai-music-companion/compare/v1.24.1...v1.25.0) (2026-05-30)

### Features

* **score:** MusicXML emitter — Phase 2 foundation ([#121](https://github.com/perice-pope/ai-music-companion/issues/121)) ([dd908aa](https://github.com/perice-pope/ai-music-companion/commit/dd908aad7cd3bf856f9ef95b8080645070b109c3))

## [1.24.1](https://github.com/perice-pope/ai-music-companion/compare/v1.24.0...v1.24.1) (2026-05-30)

### Bug Fixes

* **desktop:** resolve bundled profiles in packaged builds ([#112](https://github.com/perice-pope/ai-music-companion/issues/112)) ([#120](https://github.com/perice-pope/ai-music-companion/issues/120)) ([6dbb926](https://github.com/perice-pope/ai-music-companion/commit/6dbb9268d813fd6bd3dcab2a923b43d9b791da44))

## [1.24.0](https://github.com/perice-pope/ai-music-companion/compare/v1.23.0...v1.24.0) (2026-05-30)

### Features

* **coaching:** score title in live whispered tips ([#119](https://github.com/perice-pope/ai-music-companion/issues/119)) ([30600ff](https://github.com/perice-pope/ai-music-companion/commit/30600ff7c3e47a97fda5c3e06886264c7d01826e))

## [1.23.0](https://github.com/perice-pope/ai-music-companion/compare/v1.22.0...v1.23.0) (2026-05-30)

### Features

* **score-mode:** live cursor smoothing + measure-aware recaps (PR 3) ([#118](https://github.com/perice-pope/ai-music-companion/issues/118)) ([8432736](https://github.com/perice-pope/ai-music-companion/commit/843273693d38dc0fd068791cfdbaca85e0f8b56a))

## [1.22.0](https://github.com/perice-pope/ai-music-companion/compare/v1.21.1...v1.22.0) (2026-05-30)

### Features

* **score-mode:** live score following with on-screen cursor (PR 2 + 2.2) ([#117](https://github.com/perice-pope/ai-music-companion/issues/117)) ([c39d492](https://github.com/perice-pope/ai-music-companion/commit/c39d49294242d3302580892fb82023d20057d660))

## [1.21.1](https://github.com/perice-pope/ai-music-companion/compare/v1.21.0...v1.21.1) (2026-05-30)

### Bug Fixes

* **brain:** extract ScoreRow type alias to satisfy clippy type_complexity ([#113](https://github.com/perice-pope/ai-music-companion/issues/113)) ([7c4bdfb](https://github.com/perice-pope/ai-music-companion/commit/7c4bdfb9cd9d95815a96a905db96dd0a9e6afc16))

## [1.21.0](https://github.com/perice-pope/ai-music-companion/compare/v1.20.1...v1.21.0) (2026-05-30)

### Features

* **story-score-mode:** score library backend — PR 1 ([#96](https://github.com/perice-pope/ai-music-companion/issues/96)) ([0c1219e](https://github.com/perice-pope/ai-music-companion/commit/0c1219e0aa9eba0e2972b62887c7e4302d2bf319))
* **story-score-mode:** score picker UI + library list + load-into-session — PR 1 ([#98](https://github.com/perice-pope/ai-music-companion/issues/98)) ([c0dc00a](https://github.com/perice-pope/ai-music-companion/commit/c0dc00a0a43825e96fd9802858bc72504881fd27)), closes [#96](https://github.com/perice-pope/ai-music-companion/issues/96)

### Bug Fixes

* **hotspot-89:** eliminate vec allocation in DTW alignment hot path ([#97](https://github.com/perice-pope/ai-music-companion/issues/97)) ([655eec1](https://github.com/perice-pope/ai-music-companion/commit/655eec1de961e11705398c8f288c867d8fc9807d))

## [1.20.1](https://github.com/perice-pope/ai-music-companion/compare/v1.20.0...v1.20.1) (2026-05-01)

### Bug Fixes

* **phrase:** propagate ScorePosition into PhraseSummary when follower is set ([#94](https://github.com/perice-pope/ai-music-companion/issues/94)) ([8472042](https://github.com/perice-pope/ai-music-companion/commit/84720424b09a1b9173457c47f06def8f563c8d66)), closes [#90](https://github.com/perice-pope/ai-music-companion/issues/90) [#4](https://github.com/perice-pope/ai-music-companion/issues/4) [#91](https://github.com/perice-pope/ai-music-companion/issues/91)
* **scoring:** return neutral rhythmic_stability placeholder until follower wires up ([#93](https://github.com/perice-pope/ai-music-companion/issues/93)) ([26e92ad](https://github.com/perice-pope/ai-music-companion/commit/26e92adefb28aaa1b6194cbcd25800d3040bad17)), closes [#90](https://github.com/perice-pope/ai-music-companion/issues/90)

## [1.20.0](https://github.com/perice-pope/ai-music-companion/compare/v1.19.2...v1.20.0) (2026-05-01)

### Features

* **story-14:** implement real-time whispered tips generation ([#87](https://github.com/perice-pope/ai-music-companion/issues/87)) ([6ac27e5](https://github.com/perice-pope/ai-music-companion/commit/6ac27e53cada5df4833b07117f426a656804065d)), closes [#14](https://github.com/perice-pope/ai-music-companion/issues/14) [#21](https://github.com/perice-pope/ai-music-companion/issues/21)

## [1.19.2](https://github.com/perice-pope/ai-music-companion/compare/v1.19.1...v1.19.2) (2026-05-01)

### Bug Fixes

* **pitch-display:** remove green/yellow/red coloring — enforce neutral meter ([#86](https://github.com/perice-pope/ai-music-companion/issues/86)) ([0779358](https://github.com/perice-pope/ai-music-companion/commit/07793588d216b33edfd7e514c6434ece52ab527e)), closes [#14](https://github.com/perice-pope/ai-music-companion/issues/14)

## [1.19.1](https://github.com/perice-pope/ai-music-companion/compare/v1.19.0...v1.19.1) (2026-05-01)

### Bug Fixes

* **hotspot-34:** integrate score follower into phrase aggregator ([#85](https://github.com/perice-pope/ai-music-companion/issues/85)) ([0bf9f64](https://github.com/perice-pope/ai-music-companion/commit/0bf9f642f899422fe2acc993ab011a17cb9c6d44)), closes [#34](https://github.com/perice-pope/ai-music-companion/issues/34) [#34](https://github.com/perice-pope/ai-music-companion/issues/34)

## [1.19.0](https://github.com/perice-pope/ai-music-companion/compare/v1.18.0...v1.19.0) (2026-04-30)

### Features

* **tauri:** grant core:default capability so frontend receives emitted events ([08cff87](https://github.com/perice-pope/ai-music-companion/commit/08cff8720d0b5f48ab42feb95ebf595385ef532b))

### Bug Fixes

* **audio:** only drain ringbuf when a full detector window is buffered ([f33d95f](https://github.com/perice-pope/ai-music-companion/commit/f33d95ff65777a4a442ef9cd954417507ed7785c))

## [1.18.0](https://github.com/perice-pope/ai-music-companion/compare/v1.17.3...v1.18.0) (2026-04-30)

### Features

* **macos:** add Info.plist with NSMicrophoneUsageDescription ([9408b2d](https://github.com/perice-pope/ai-music-companion/commit/9408b2d6375719c2bfd052df22b1a8631c08a645))

## [1.17.3](https://github.com/perice-pope/ai-music-companion/compare/v1.17.2...v1.17.3) (2026-04-27)

### Performance Improvements

* **hotspot-89:** eliminate Vec allocation in score follower DTW alignment step ([e1219f8](https://github.com/perice-pope/ai-music-companion/commit/e1219f8cecf816a48a5de28c650b85cf95f9675d))

## [1.17.2](https://github.com/perice-pope/ai-music-companion/compare/v1.17.1...v1.17.2) (2026-04-25)

### Bug Fixes

* **hotspot-6:** make pitch meter thresholds profile-based instead of hard-coded ([4f72189](https://github.com/perice-pope/ai-music-companion/commit/4f72189df60131446d6b9142b7394216bd82c623))

## [1.17.1](https://github.com/perice-pope/ai-music-companion/compare/v1.17.0...v1.17.1) (2026-04-24)

### Bug Fixes

* address CTO audit hotspots — error handling, deps, testing patterns ([efeae0b](https://github.com/perice-pope/ai-music-companion/commit/efeae0b418aeec927677c743a8914ecf1c8f774f)), closes [#64](https://github.com/perice-pope/ai-music-companion/issues/64)

## [1.17.0](https://github.com/perice-pope/ai-music-companion/compare/v1.16.0...v1.17.0) (2026-04-24)

### Features

* **audio:** live pitch IPC — mic → pitch detector → audio-event ([#83](https://github.com/perice-pope/ai-music-companion/issues/83)) ([f2dd302](https://github.com/perice-pope/ai-music-companion/commit/f2dd302a113917f4241ad25ed6b11a829e6bd503))

## [1.16.0](https://github.com/perice-pope/ai-music-companion/compare/v1.15.0...v1.16.0) (2026-04-24)

### Features

* **story-21:** practice mode UI — selector + in-session switcher ([#82](https://github.com/perice-pope/ai-music-companion/issues/82)) ([d699984](https://github.com/perice-pope/ai-music-companion/commit/d6999842ae18c15f8bb3b948e0913f4d8dedf1a9)), closes [#81](https://github.com/perice-pope/ai-music-companion/issues/81)

## [1.15.0](https://github.com/perice-pope/ai-music-companion/compare/v1.14.0...v1.15.0) (2026-04-24)

### Features

* **story-14:** wire up LLM recap generation to Tauri command surface ([#79](https://github.com/perice-pope/ai-music-companion/issues/79)) ([9d8984a](https://github.com/perice-pope/ai-music-companion/commit/9d8984ae53ffda77f2a2686838821793f057ed00))
* **story-21:** practice mode infrastructure — Warmup, Practice, RunThrough ([#81](https://github.com/perice-pope/ai-music-companion/issues/81)) ([e44c340](https://github.com/perice-pope/ai-music-companion/commit/e44c3402a11aba20aeab7a5c7fe56f567bad9d45))

## [1.14.0](https://github.com/perice-pope/ai-music-companion/compare/v1.13.1...v1.14.0) (2026-04-24)

### Features

* **story-14:** implement RecapGenerator for CoachingEngine ([#78](https://github.com/perice-pope/ai-music-companion/issues/78)) ([1e956f9](https://github.com/perice-pope/ai-music-companion/commit/1e956f98b8eca0088b91424cfd3865d6db6f338e)), closes [#14](https://github.com/perice-pope/ai-music-companion/issues/14)

## [1.13.1](https://github.com/perice-pope/ai-music-companion/compare/v1.13.0...v1.13.1) (2026-04-23)

### Bug Fixes

* **hotspots:** replace eprintln! with tracing to fix Windows silent drops and deadlock risk ([#74](https://github.com/perice-pope/ai-music-companion/issues/74)) ([5e40cb2](https://github.com/perice-pope/ai-music-companion/commit/5e40cb27226af13292cab5250157e6472b2cae64))

## [1.13.0](https://github.com/perice-pope/ai-music-companion/compare/v1.12.0...v1.13.0) (2026-04-23)

### Features

* **profiles:** add 6 missing instrument profiles (trombone, french-horn, cello, flute, clarinet, piano) ([#73](https://github.com/perice-pope/ai-music-companion/issues/73)) ([64f6c4c](https://github.com/perice-pope/ai-music-companion/commit/64f6c4cf6b83f2c0b28e588774924d19d4c2aa3a)), closes [#3](https://github.com/perice-pope/ai-music-companion/issues/3)

## [1.12.0](https://github.com/perice-pope/ai-music-companion/compare/v1.11.1...v1.12.0) (2026-04-23)

### Features

* **story-14:** PR 3 — real LLM coaching and graceful degradation ([#68](https://github.com/perice-pope/ai-music-companion/issues/68)) ([23714d5](https://github.com/perice-pope/ai-music-companion/commit/23714d5fac70cd523e7e4b6eef3f3e521df919d7))

### Bug Fixes

* **hotspots:** prevent NaN panic in audio thread + align PitchDisplay with v2 design ([#69](https://github.com/perice-pope/ai-music-companion/issues/69)) ([863a8e4](https://github.com/perice-pope/ai-music-companion/commit/863a8e44cb7ff734f2823b4bcb71047a80025871))

## [1.11.1](https://github.com/perice-pope/ai-music-companion/compare/v1.11.0...v1.11.1) (2026-04-22)

### Bug Fixes

* **scoring:** replace per-note verdicts with phrase-level assessment ([#63](https://github.com/perice-pope/ai-music-companion/issues/63)) ([a4f5efc](https://github.com/perice-pope/ai-music-companion/commit/a4f5efc13adea0b6a6ec68ae959bb271ecee3821)), closes [#32](https://github.com/perice-pope/ai-music-companion/issues/32)

## [1.11.0](https://github.com/perice-pope/ai-music-companion/compare/v1.10.0...v1.11.0) (2026-04-22)

### Features

* **coaching:** instrument-specific prompts and enhanced testing ([#61](https://github.com/perice-pope/ai-music-companion/issues/61)) ([720287f](https://github.com/perice-pope/ai-music-companion/commit/720287f75a7238ee9795a7a170095bb5d0446318))

## [1.10.0](https://github.com/perice-pope/ai-music-companion/compare/v1.9.0...v1.10.0) (2026-04-22)

### Features

* **story-17:** Practice history + progress dashboard ([#57](https://github.com/perice-pope/ai-music-companion/issues/57)) ([4d3b001](https://github.com/perice-pope/ai-music-companion/commit/4d3b0018347eb54af25d77328a009697cf3b3c3f)), closes [#17](https://github.com/perice-pope/ai-music-companion/issues/17) [#17](https://github.com/perice-pope/ai-music-companion/issues/17)

## [1.9.0](https://github.com/perice-pope/ai-music-companion/compare/v1.8.0...v1.9.0) (2026-04-21)

### Features

* **ci:** throttle daily agent — 4×/day with 3-open-draft cap (clean rebase) ([#56](https://github.com/perice-pope/ai-music-companion/issues/56)) ([a8dade4](https://github.com/perice-pope/ai-music-companion/commit/a8dade4c4b8f5f43bcc5d106dd52ebc07eed00a8)), closes [#3](https://github.com/perice-pope/ai-music-companion/issues/3) [#4-10](https://github.com/perice-pope/ai-music-companion/issues/4-10)

## [1.8.0](https://github.com/perice-pope/ai-music-companion/compare/v1.7.0...v1.8.0) (2026-04-21)

### Features

* **story-16:** Online DTW score follower — Part 1 ([#53](https://github.com/perice-pope/ai-music-companion/issues/53)) ([6943581](https://github.com/perice-pope/ai-music-companion/commit/69435810e3421d85b864d802f898e70e3944f973)), closes [#16](https://github.com/perice-pope/ai-music-companion/issues/16) [#16](https://github.com/perice-pope/ai-music-companion/issues/16)

## [1.7.0](https://github.com/perice-pope/ai-music-companion/compare/v1.6.0...v1.7.0) (2026-04-21)

### Features

* **story-14:** PR 2 — coaching tip panel with auto-dismiss animation ([#51](https://github.com/perice-pope/ai-music-companion/issues/51)) ([ca3527e](https://github.com/perice-pope/ai-music-companion/commit/ca3527e96d30b655356e79e96f67488132540bde)), closes [#14](https://github.com/perice-pope/ai-music-companion/issues/14)

## [1.6.0](https://github.com/perice-pope/ai-music-companion/compare/v1.5.3...v1.6.0) (2026-04-21)

### Features

* **ci:** default daily agent to Haiku, allow Sonnet via manual dispatch ([#47](https://github.com/perice-pope/ai-music-companion/issues/47)) ([f555c6f](https://github.com/perice-pope/ai-music-companion/commit/f555c6fd5f79ec8c35f1d24732d83eb095f4341e))
* **story-14:** PR 1 — scaffolding, timer, mock pipeline ([#48](https://github.com/perice-pope/ai-music-companion/issues/48)) ([b2ada68](https://github.com/perice-pope/ai-music-companion/commit/b2ada68843023f7e4ce727a2fc29e934496b825f))

## [1.5.3](https://github.com/perice-pope/ai-music-companion/compare/v1.5.2...v1.5.3) (2026-04-21)

### Bug Fixes

* **ci:** commit pnpm-lock.yaml, drop sloppy --frozen-lockfile fallback ([#46](https://github.com/perice-pope/ai-music-companion/issues/46)) ([4861748](https://github.com/perice-pope/ai-music-companion/commit/4861748bc01046bef9b0285aa6837018745dc54d)), closes [#44](https://github.com/perice-pope/ai-music-companion/issues/44)

## [1.5.2](https://github.com/perice-pope/ai-music-companion/compare/v1.5.1...v1.5.2) (2026-04-21)

### Performance Improvements

* **ci:** trim apt install list + better caching (~10min off cold runs) ([#44](https://github.com/perice-pope/ai-music-companion/issues/44)) ([ae85d01](https://github.com/perice-pope/ai-music-companion/commit/ae85d01ee8389f9a71129e2423b8be75f1350b4d)), closes [#14](https://github.com/perice-pope/ai-music-companion/issues/14)

## [1.5.1](https://github.com/perice-pope/ai-music-companion/compare/v1.5.0...v1.5.1) (2026-04-21)

### Bug Fixes

* **ci:** add id-token write permission for claude-code-action ([#43](https://github.com/perice-pope/ai-music-companion/issues/43)) ([1498fd2](https://github.com/perice-pope/ai-music-companion/commit/1498fd2dcb568f0d9b35eecb5a0f7629b18414dc)), closes [#31](https://github.com/perice-pope/ai-music-companion/issues/31)

## [1.5.0](https://github.com/perice-pope/ai-music-companion/compare/v1.4.0...v1.5.0) (2026-04-20)

### Features

* **ci:** latency benchmark enforcing <25ms budget ([#38](https://github.com/perice-pope/ai-music-companion/issues/38)) ([b871950](https://github.com/perice-pope/ai-music-companion/commit/b87195027e446dad6f233e5ce731d800bc56bbfa))

### Bug Fixes

* preserve best takes during size-based retention sweep ([#37](https://github.com/perice-pope/ai-music-companion/issues/37)) ([d83cb3b](https://github.com/perice-pope/ai-music-companion/commit/d83cb3b0c32b1a634b6b0a6d0993f711b2089b23)), closes [#36](https://github.com/perice-pope/ai-music-companion/issues/36)

## [1.4.0](https://github.com/perice-pope/ai-music-companion/compare/v1.3.0...v1.4.0) (2026-04-20)

### Features

* metronome and tuning drone — Story [#19](https://github.com/perice-pope/ai-music-companion/issues/19) ([#30](https://github.com/perice-pope/ai-music-companion/issues/30)) ([ef8f25e](https://github.com/perice-pope/ai-music-companion/commit/ef8f25e16de9a90a186f38cfa2c5ad1103a4fc54))
* session audio recording infrastructure — Story [#20](https://github.com/perice-pope/ai-music-companion/issues/20) ([#29](https://github.com/perice-pope/ai-music-companion/issues/29)) ([bc879a8](https://github.com/perice-pope/ai-music-companion/commit/bc879a8a969f8a30890402ee34230870bd493d86))

## [1.3.0](https://github.com/perice-pope/ai-music-companion/compare/v1.2.0...v1.3.0) (2026-04-19)

### Features

* session recorder + SQLite store — Story [#13](https://github.com/perice-pope/ai-music-companion/issues/13) ([#28](https://github.com/perice-pope/ai-music-companion/issues/28)) ([3163c1c](https://github.com/perice-pope/ai-music-companion/commit/3163c1ce5fbdcbfaaf20e709dfa48648bcfa50cd)), closes [#12](https://github.com/perice-pope/ai-music-companion/issues/12) [#14](https://github.com/perice-pope/ai-music-companion/issues/14)

## [1.2.0](https://github.com/perice-pope/ai-music-companion/compare/v1.1.0...v1.2.0) (2026-04-19)

### Features

* MusicXML and MIDI score parser — Story [#15](https://github.com/perice-pope/ai-music-companion/issues/15) ([#26](https://github.com/perice-pope/ai-music-companion/issues/26)) ([787eefc](https://github.com/perice-pope/ai-music-companion/commit/787eefc146534f9c7260c8511dcfbb89507bb784))

## [1.1.0](https://github.com/perice-pope/ai-music-companion/compare/v1.0.0...v1.1.0) (2026-04-17)

### Features

* LLM coaching engine — Story [#12](https://github.com/perice-pope/ai-music-companion/issues/12) ([#25](https://github.com/perice-pope/ai-music-companion/issues/25)) ([f484ab0](https://github.com/perice-pope/ai-music-companion/commit/f484ab08daf55d245e901fd643342e04302bace8))

## 1.0.0 (2026-04-17)

### Features

* add instrument profile selector with Zustand persistence ([#23](https://github.com/perice-pope/ai-music-companion/issues/23)) ([7b3ed36](https://github.com/perice-pope/ai-music-companion/commit/7b3ed369024b8187ac02eec1c8dc45dc86d299a1))
* Brain crate phrase aggregator — Story [#11](https://github.com/perice-pope/ai-music-companion/issues/11) ([#22](https://github.com/perice-pope/ai-music-companion/issues/22)) ([947d154](https://github.com/perice-pope/ai-music-companion/commit/947d154497c07c2cd6142507805b9e58e4faba48))
* cpal mic capture with lock-free ring buffer — Story [#2](https://github.com/perice-pope/ai-music-companion/issues/2) ([#8](https://github.com/perice-pope/ai-music-companion/issues/8)) ([1201d5c](https://github.com/perice-pope/ai-music-companion/commit/1201d5cdb58b4827525b7c41180b0f1bf848a640))
* Instrument profile loader with validation — Story [#5](https://github.com/perice-pope/ai-music-companion/issues/5) ([#7](https://github.com/perice-pope/ai-music-companion/issues/7)) ([95ba2e5](https://github.com/perice-pope/ai-music-companion/commit/95ba2e59f4a83a3de0fe84fb439308db20d23159))
* live pitch display with Zustand store — Story [#4](https://github.com/perice-pope/ai-music-companion/issues/4) ([#10](https://github.com/perice-pope/ai-music-companion/issues/10)) ([a2153d2](https://github.com/perice-pope/ai-music-companion/commit/a2153d2aae8bc99e8252fd0b3d951ba23e322693))
* pure-Rust YIN pitch detector — Story [#3](https://github.com/perice-pope/ai-music-companion/issues/3) ([#9](https://github.com/perice-pope/ai-music-companion/issues/9)) ([110311e](https://github.com/perice-pope/ai-music-companion/commit/110311e6de18a0536db5ef0dbf6c6deb44b8da1d))
* scaffold AI Music Companion project ([78a1ed3](https://github.com/perice-pope/ai-music-companion/commit/78a1ed349eb084514b47704c73fdbaa0eb00d193))
* Tauri IPC ping/pong — Story [#1](https://github.com/perice-pope/ai-music-companion/issues/1) ([#6](https://github.com/perice-pope/ai-music-companion/issues/6)) ([46e92cb](https://github.com/perice-pope/ai-music-companion/commit/46e92cb2cc1defb441cea6349391da658cb6b801))

### Bug Fixes

* **ci:** use npx -p flags for semantic-release plugin resolution ([4899a3d](https://github.com/perice-pope/ai-music-companion/commit/4899a3d60cf1c0486425e5cb133e66b11c77a687))
