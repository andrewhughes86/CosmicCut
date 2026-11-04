# Changelog

## 0.5.1

- While a cut is set up with the table turned to its angle, the wire is now drawn square to the
  cut's plane (and the floor grid stays still), as in the simulation. Before, the wire turned
  with the table and crossed the outline at an angle.

## 0.5.0

### Profiles and two-views

- **They follow models too.** Set *Source* to *Models* (or right-click a model: *Profile of
  model…*, *Two-view of model…*) and the wire follows the outline of the chosen models seen
  along the wire: one view for a profile and its copies, front and side for a two-view.
- **Contours are picked as chains, as in Fusion.** While a profile or two-view is set up, the
  models go see-through and the outline is drawn; nothing is picked to start with. Click lines
  one at a time: a line that meets either end of the chain being picked carries it on, any other
  starts a new chain, and clicking a picked line takes it out (a chain broken in the middle
  becomes two). Alt-click picks a line's whole run at once. Picked lines are purple (#6A06D3).
- **Every chain has a direction.** Click its arrow, or *Flip* in the panel, to cut it the other
  way. Chains are cut in the order picked.
- **Lead-in and lead-out lengths**: straight leads along the start and end of each chain.
- **A model's bottom is not cut** unless you pick it; bottoms are drawn dotted.
- **Bridges along Y0.** Where picked chains of two models end on the floor next to each other,
  the wire runs along machine Y0 from one to the next instead of leaving the foam (*Bridge along
  Y0*, on by default). It never bridges or leaves along the floor under a model, nor up inside a
  wall's kerf; when no straight way out is clear it goes along the floor into the gap, then up.
- **Two-view: Front / Side.** A two-view shows one view at a time: the table turns to it and
  only its lines are drawn and picked. Each chain is listed with its view.
- While a cut is made or changed, the other cuts' paths are hidden and the table is drawn
  turned to the cut's angle.
- Older profile cuts open with every line but the models' bottoms picked, as they were cut.

### The cut panel

- Settings in folding groups (Geometry, Table, Placement, Passes, Linking…), two aligned
  columns, units beside the values, and what a blank field stands for shown in it. The notes
  are gone: what a setting does shows when the pointer rests on its name for a second.
- Settings a chain decides (entry side, start point, direction, bottom) are gone from profiles.

### Simulation

- **Each part comes out as its own body.** A profile's cuts split the foam into pieces: those
  holding part of a model (or of the shape cut) stay, the rest is waste and goes, even where
  parts stand closer together than the simulation's cells. Any loose piece with no part in it
  falls away too. Before, a run of profiles could keep the waste joining the parts, or leave
  nothing.
- A model an operation (or cut) says is off the table is set aside whole: later cuts no longer
  eat into it in the simulation.
- **Exit simulation** button at the top centre of the viewport.

### Explorer and viewport

- **Select several.** Ctrl-click cuts, models and shapes in the explorer (or models in the
  viewport) to add them to the selection or take them out; Shift-click selects every row from
  the last one selected. Delete (key or right-click) deletes them all at once, in one step that
  one Undo brings back.
- **Drag cuts** to put them before or after another cut, or onto an operation to make them its
  last cut. A line shows where it will go (`cut.place` from the console).
- **Stock and Origin can be hidden** with their eyes in the explorer (or the View options). The
  origin shows in the viewport as a marker at the program's X0 Y0.
- **The delete question stands out**: larger, the window behind it dimmed, glowing purple in the
  dark theme (red in the light one). Enter confirms it.

## 0.4.1

- **Origin in the explorer.** The program's X0 Y0 can be machine zero (as before) or the
  turntable: X0 on its axis, Y0 at the table top. Right-click *Origin* to choose, or
  double-click it to switch. It is saved with the job, and the G-code says which one it uses.
- **Table turns as G1.** A post processor setting writes the moves that turn the table as G1 at
  a feed you give (written as the F word as it is, degrees per minute on most controllers)
  instead of G0. The cut after a turn gets its own feed back.
- Swarf: a wall that meets a ledge in a corner no longer counts as cutting into the part. A
  ledge beside a slanted wall is swept along the wall. Ledges that do not touch are swept
  separately.

## 0.4.0

- The download files carry the version in their names, e.g. `CosmicCut-0.4.0-windows-x64-setup.exe`.
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
