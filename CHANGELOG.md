# Changelog

## 0.2.0

- **Swarf cuts.** Click faces of a model and the wire lies along them at every height, with X,
  Y and B moving together. They suit tapered, twisted and turning faces made of level straight
  lines.
  - A click takes the whole surface the wire can follow, even when the model splits it into
    many small faces. Alt-click takes a single face.
  - Faces that line up are cut in one pass, and the rest get a pass each.
  - The kerf is measured square to the face.
  - The feed is slowed so no point of the wire across the foam moves faster than the cut's feed.
  - The cut warns if the wire would cut into another part of the model.
- **Start page.** CosmicCut opens on New project, Open, the imports, recent projects and the
  sample. **File → Home** brings it back.
- **Recent projects.** The last 8 jobs opened or saved are remembered.
- New sample: `samples/twisted-block.stl`, a block with twisted sides for trying swarf cuts.

## 0.1.0

- First release: Windows, macOS and Linux installers, and the browser version.
- Profile, two-view, lathe, slab and from-model cuts.
- A sketcher with constraints and dimensions.
- Simulation with the turntable turning.
- Machine and post processor libraries.
- The wire never goes below Y0, which is now the table top by default.
- The table turns square (B0 or B180, whichever is nearer) after the last cut, then the wire
  goes home.
