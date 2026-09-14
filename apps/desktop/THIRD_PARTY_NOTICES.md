# Desktop third-party assets

The default SVG character is a project prototype asset. Optional Live2D assets are prepared separately, are not checked into Git, and are governed by their own licenses.

| Component | Version/source | Terms/location |
|---|---|---|
| PixiJS and CSP compatibility patch | npm `pixi.js` and `@pixi/unsafe-eval` 6.5.10 | MIT; package LICENSE files |
| pixi-live2d-display browser runtime | 0.4.0 | MIT; local downloaded LICENSE preserved |
| Live2D Cubism Core | Pinned reference repository commit in manifest | Live2D proprietary SDK license; local Core LICENSE preserved |
| Hiyori sample | Pinned official CubismWebSamples commit in manifest | Live2D SDK/sample terms; local LICENSE and NOTICE preserved |

See the [asset manifest](scripts/live2d-sample-manifest.json) for pinned locations and checksums and the [setup guide](../../docs/development/model-settings-live2d.md) for restrictions. The reference project's MIT license does not grant commercial rights to third-party SDK or character assets.

Local builds include files present in `public/live2d-local`. Confirm applicable redistribution and commercial rights before distributing a build containing those files. Current prepared samples are for internal technical evaluation; this document does not claim clearance for commercial release.