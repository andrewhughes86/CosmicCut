# Changelog

## 0.4.0

- **A swarf cut takes faces from any models,** not only the selected one: click faces on any
  model in the viewport. Each model's passes keep clear of the others in the cut and of every
  other model, unless they are listed as off the table.
- **Swarf passes leave by the way that is clear.** At the bottom of its faces the wire goes out
  level through the waste when nothing is in the way. If a part is in the way, it backs up the
  kerf it came down (cutting nothing new) and leaves above the foam. For example, an end part's
  inner face can be cut without the wire running into the parts in the middle.
- **Swarf cuts level faces too.** The wire sits level at the face, half a kerf off on the waste
  side, and sweeps in from outside the foam to the far edge of the face and back out. It uses
  the table angle and side with the shortest way in that keeps clear of everything else. Level
  faces are cut last, from the top down, so a part's bottom is cut free at the very end. A face
  on the table, with no foam under it, is left alone with a note.
- Every swarf pass must keep out of the part itself as well as other models (unless they are
  listed as off the table). A pass that would cut into the part is now an error, not a warning.
- **Seen from below, the turntable is see-through**, so a part's bottom face can be seen and
  picked.

## 0.3.0

- **Several models in one cut.** A From model cut takes any of the job's models (all of them by
  default) and follows the outline of them all together at each angle.
- **The wire keeps out of other models.** Every cut is checked against the models it is not
  making. Because the wire runs right across the foam, a cut fails with an error naming the
  model, the table angle and the height if the wire or its kerf would pass through one.
- **Move tool** (Home → Move): arrows slide the model along X, Y and Z, and rings turn it about
  them. After a drag the value shows in a box, selected, to type over or accept with Enter. Each
  model keeps its position and turns (now about X and Y as well as Z), and saves them with the
  job.
- **Click a model in the viewport** to select it (the selected one is tinted). **Ctrl+D**
  duplicates the selected cut or model, and Ctrl+C then Ctrl+V copies a model. A copied model
  stands beside the original, not on top of it.
- **Models already off the table.** An operation's settings, or a cut's, can list models that
  were cut and taken off before it runs. The wire may pass through those, and either list is
  enough. For example, cut the parts at the ends of a block in Op 1, take them off, then cut the
  centre parts in Op 2 with the end parts listed.
- **Profile cuts can leave lines out.** In the Cut tab, **Leave out lines**, then click lines
  of the sketch in the viewport (left-out lines show dashed; click one again to put it back).
  - What is left is cut as it is, from the end nearest the start point (**Pick the start
    point**).
  - Where an end is inside the foam, the wire comes in level from the nearest side that does
    not cross the outline, or down from the top, and leaves the same way.
  - For example, leave out a part's bottom line and start at its outer corner: the wire goes
    round the part and parts it off along the bottom back out to the outside.
  - The simulation keeps the side of the outline that holds the part.
- Swarf faces that are picked stay highlighted in their own colour until the Cut tab closes.
  Highlighted faces now show through the stock.

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
