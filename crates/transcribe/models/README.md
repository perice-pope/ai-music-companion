# Bundled model: `nmp.onnx`

This is the **basic-pitch** note-prediction model from Spotify, vendored here
for offline audio-to-MIDI transcription.

- **Source:** <https://github.com/spotify/basic-pitch> —
  `basic_pitch/saved_models/icassp_2022/nmp.onnx` (a `tf2onnx` export of the
  original TensorFlow model).
- **License:** Apache License 2.0 (© Spotify AB). The model and basic-pitch's
  note-creation algorithm (ported in `src/notes.rs`) are used under that
  license.
- **Size / format:** ~225 KB ONNX. Single input
  `serving_default_input_2:0` (`f32[batch, 43844, 1]`), three outputs:
  `StatefulPartitionedCall:1` = note, `:2` = onset, `:0` = contour
  (`[batch, 172, 88|264]`).

The model is embedded into the crate at compile time via `include_bytes!`, so
no runtime resource resolution is required. Updating it is a matter of replacing
this file. The native ONNX **Runtime** is a separate concern — see the crate
docs and `.github/workflows/ci.yml`.
