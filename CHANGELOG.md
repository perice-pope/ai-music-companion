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
