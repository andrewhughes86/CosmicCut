# CosmicCut

CAM for hot-wire foam cutters with a wire that moves in **X** (across) and **Y** (up) over a
**B-axis turntable**. Import or make 2D shapes, set up cuts, watch the foam come away in the
simulation, and post G-code for the machine.

The interface, and the code behind it, come from CosmicDMS: the same ribbon, dockable
workspace, theme, icons, viewport and command system.

## Run

    cargo run --release

It opens with a sample job: a low-poly penguin (`samples/penguin.stl`) cut from a foam block
with a profile every 22.5°. **Sample** in the job's right-click menu opens it again. `--empty` starts with an empty job;
`CosmicCut job.ccut` opens a job. `--help` lists the other options.

## Menus

The menu bar above the ribbon holds **File** (New, Open, Save, Save as, Close, which asks
before dropping unsaved changes, the imports, Post G-code, Preferences, Exit), **Edit** (Undo
Ctrl+Z, Redo Ctrl+Y, and Cut, Copy and Paste of the selected cut or shape: a pasted cut goes
into the active operation; models copy too), **View** (show or hide the Explorer, Inspector, Cut plane, G-code
and Console panels and the viewport's grid, view cube, axes, stock, machine and rulings; the
theme; Reset layout) and **Help** (About). Undo keeps the last 100 changes to the job.

**Several jobs at once.** Each open job has a tab in the menu bar. New, Open and the sample
open a new tab (Open replaces an untouched empty job, and brings a file that is already open
to the front). Click a tab to switch to it, or press Ctrl+Tab for the next one; **+** starts a new
job. A tab shows its close × while the pointer is over it, and a dot after the name marks
unsaved changes. Each job keeps its own undo history, selection, view and simulation.
Closing the last tab leaves an empty job.

## A job, start to finish

The Home tab runs left to right in the order a job goes: **New sketch**, **Import model**,
**Import sketch**, **Stock**, **Machine**, then the cuts, operations, **Simulation** and posting.
New, Open, Save (Ctrl+S) and Save as (Ctrl+Shift+S) are in the **File** menu. The panel under
the viewport has two tabs, **Console** and **Instructions** (what the tool in use wants next);
the one last used is in front when CosmicCut opens.

1. **Machine**: the **Machine** button opens this job's machine in the **Machine** tab beside
   the cut plane (Save machine applies it); **Machine library** (Machine tab, in its own window) keeps your machines between jobs: choose the one a job runs on, add, copy, rename and
   delete them. Each machine names its **post processor**, chosen from the **Post processor
   library**, which comes with HotwireCNC, LinuxCNC (mm), Mach3 / Mach4 and a generic post, and
   takes your own (a copy of any of them, then **Edit**). The defaults match the
   HotwireCNC machine: 100 × 55 in travel, the turntable's axis at X 50, the wire 0.5 in above
   the table at Y0, a 0.033 in kerf, programs in inches.
   Out of the simulation the wire waits at home, machine X0 Y0.
2. **Stock** opens the **Stock** tab beside the cut plane and the G-code: a **Block** (width,
   depth, height) or **Round** (diameter, height), where it stands on the table, how high (a
   riser), and **Fit to model**. While it is open the viewport hides the toolpaths and draws
   the stock's dimensions; click a dimension to type a new value there. Values apply as they
   are entered (Enter, or leaving the field), and Undo takes them back.
3. **Models**: **Import model** reads STL, OBJ, 3MF, PLY, STEP, IGES, SAT and the other formats
   CosmicDMS reads. The model is always centred on the turntable's axis, standing on the stock's
   bottom. **Bottom face** (Home, or double-click the model) lets you click the face to put
   down on the table; the model turns that face down and centres itself again. **Centre on
   table** frees it to stand anywhere (set where in **Position**) for parts that do not need
   the turntable. **Scale** makes it as large as fits in the stock (with a margin round the
   sides and top), a given height, or a factor of its file size. **Turn and lift** turns it on
   the table or raises it off the stock's bottom.
   **From model** (in the cuts gallery, or right-click the model) is the one cut for a model:
   the table turns between cuts, and at every rotation the wire follows a new profile, the
   model's outline seen from that angle. Space the cuts by dividing 180° into a number of
   cuts, or by degrees apart (180° is enough: the outline at 180° is the one at 0°,
   mirrored). The cut follows the model, so laying it flat, turning or scaling it updates the
   cut.

   Anything lower than the wire reaches is left joined to the foam underneath (put the stock
   on a riser, or lift the model, to cut it free).
4. **Sketches**: **Import sketch** (Sketch or Home tab) reads DXF (lines, arcs, circles,
   polylines with bulges, splines, ellipses), SVG (paths, polygons, rects, circles, with
   transforms), airfoil `.dat` (Selig or Lednicer) and XY point lists; **NACA airfoil** makes a
   four-digit airfoil. Each comes in as a sketch on the front plane, a line between each pair of
   its points, so it takes constraints and dimensions like anything drawn (a shape from an older
   job becomes one when you draw in it). Select one to see it in the **Cut plane** pane.
   The **Sketch** tab draws your own shapes; New sketch on Home goes there. **New sketch** shows the three planes through the
   turntable's axis in the viewport, **Front** (X across, Z up: the plane the wire cuts in at
   B = 0), **Right** (Y, Z) and **Top** (X, Y); click one (Esc cancels). The view turns square to
   it and frames the stock, a grid is drawn on the plane (lines every 4 in, or 100 mm, with
   quarters between), and the wire waits at home (X0 Y0) out of the way. Then:
   - **Create**: Line (end to end; Esc or right-click stops; while it goes on, **R** makes the
     next segment an arc tangent to it and **C** a 45° chamfer turning towards the mouse), Rectangle, Centre rectangle,
     Circle (centre, then edge), Arc (start, end, then a point on it), Polygon (centre, corner),
     Point. Points snap to end points (shared), midpoints and curves, shown by a ring, a
     triangle and a cross as you move; end points reach further than curves, so a ring shows
     as you near one and a click takes it. A line drawn nearly level or upright is held that way
     (H or V shows by the cursor). A point lined up level with, or straight above, another point
     (the origin's are the plane's axes, so Z0 and the turntable's axis) snaps there, shown by a
     dashed guide. Each snap becomes a constraint.
   - **Constrain** what is selected (click; Ctrl or Shift adds; constraints that do not suit
     what is selected are greyed out): horizontal, vertical, coincident,
     parallel, perpendicular, equal, tangent, midpoint, concentric, fix. Each shows as a small
     mark beside what it holds. Points, centres, the origin and the middles of lines can all be
     picked (a ring shows round them as you near them), so e.g. a line's middle can be put
     straight above the origin (select both, Vertical) or dimensioned from it.
   - **Dimension** (D): click a line (length), circle (diameter) or arc (radius) and its
     dimension shows at once, following the mouse; click a second item to make it the distance
     or angle between the two (two lines: the angle where they meet, or the gap if parallel).
     Click empty space to put it there, then type the size: the value is selected, so typing
     replaces it. Click any size to change it, drag it to move it, right-click it to
     delete it. The solver moves the sketch to fit;
     anything that cannot be met (over-constrained, or a line shrunk to nothing) is refused and
     nothing changes.
   - Drag points and curves in Select; they move as far as their constraints let them. Drag
     on empty space for a selection window: left to right takes what is wholly inside it,
     right to left anything it touches. **Ctrl+A** selects everything; Esc lets go. Drag any
     selected item to move the whole selection together (the rest follows its constraints).
     **Move** (M) drags the selection, or the whole sketch, the same way.
     **Construction** makes guides that are not part of the outline; **Delete** removes items.
     The **Instructions** tab says what the tool wants next, and how many degrees of freedom
     are left (the sketch turns from cyan to white when fully constrained).
   - **Finish sketch** (on the ribbon, or top centre of the viewport) goes back to Home. The
     sketch is a shape (under Shapes in the explorer) its closed outlines cut from; cuts
     that use it follow when it changes. Double-click it to draw in it again.

5. **Cuts** (the gallery on Home and Cuts):
   - **Profile**: follow a shape through the foam at one table angle, or at several (Copies and
     Step) for indexed parts. Keep the inside (a part) or the outside. The wire enters from the
     left, right or top and leaves through the same kerf. **Pick the start point** (in the Path
     card) and click in the viewport where the cut should start: the wire goes into the outline
     at its point nearest there, from the side that reaches it without crossing the part (with
     the bottom attached, the side nearer the pick). **Clear** goes back to the lead side.
   - **Two-view**: a front shape, a quarter turn of the table, a side shape. The part is what
     both keep.
   - **Lathe**: a half profile (radius against height) cut at many table angles, for vases,
     columns and finials. Each pass runs down the profile and out at the bottom; the table turns
     with the wire above the foam.
   - **Slabs**: level passes that slice the block into sheets, with an optional skim off the top.

   **The part stays on the table.** Profile and two-view cuts come in level at the part's
   bottom, go over its top and leave level on the other side, so the part stays joined to the
   foam under it while the table turns. The program then ends with the **release**: the last
   cut that holds the part follows its bottom (a level cut under the foot, for lathe cuts) and
   cuts it free. It is drawn in red on the selected cut. *Attach* raises the level the part is
   held at; *Bottom: through* and *Release: no* go back to cutting right round it.

   **Operations.** The explorer lists the cuts under **Op 1**. **Add operation** starts Op 2
   for cuts that come after a machine stop, for example once the operator has turned the stock
   by hand. **Operation settings** sets its stop (none, `M0` or `M1`), a message the controller
   shows (`(MSG, …)`) and notes written into the program; the wire is off while the machine
   waits, and the simulation pauses there too. New cuts go into the selected operation;
   right-click a cut to move it to another.

   Each opens in the **Cut** tab, beside the cut plane and the G-code, with a live preview in the viewport and the cut plane.
   Kerf compensation runs the wire half a kerf off the part.
6. **Simulation**: plays the whole program. The turntable turns with the foam on it while the
   wire and its towers stay put, as on the machine. The waste falls away as each pass finishes, so the
   end shows the finished part. The wire's X, Y and B are shown as it moves.
7. **Post G-code** writes the program; **View G-code** shows it in the G-code pane.
   **Post processor** (Machine tab) opens the machine's post in the **Post processor** tab beside
   the cut plane (Save post applies it to every machine that uses it; **View G-code** shows the
   result): the start (safety) block and end code, the wire on and off codes and preheat,
   wire off between cuts and code between cuts (e.g. `M0`), a G0/G1 on every move line, the
   feed on every line, arcs, the turntable's axis letter, units, decimals, line numbers, comments,
   `%` wrappers, the file extension, and the table turned to an angle after the last cut (B180 unless changed: wire off, out past where the foam swings, then the turn), the return home at the end (wire off, then up to the
   top of the travel, across to X0 and down when the part is in the way, else straight to X0 Y0).
   Decimals you set are always written in full, trailing zeros too. In the code blocks `{units}` becomes G20 or G21, `{job}`
   the job's name and `{preheat}` the preheat seconds. The libraries are kept with your settings; a job keeps a copy of its machine and post, so
   it posts the same way on another computer.

Every button runs a command, and the console takes the same commands:
`cut.slab {"thickness": 50.8, "skim": 6.35}`. Commands take millimetres and degrees whatever
the display units.

## Frames and the G-code

Inside, everything is in the **table frame**: the origin is the turntable's axis on the table
top, Z is up, and at B = 0 the wire runs along Y. A wire position is `u` (across, from the axis)
and `h` (height above the table). The viewport draws the table frame, turned by B
while simulating, so the turntable turns and the wire stays still.

By default the post writes RS274/NGC for LinuxCNC-style controllers: `G17 G90 G91.1 G40 G49 G94`,
`G20` or `G21`, `M3` and a 2 s preheat dwell, `G0`/`G1` with `X Y B`, `G2`/`G3` where the path
follows circles, `M5 M2`; all of it can be changed in **Post processor**. Machine X = table X + u, machine Y = h − (wire height at Y0).
The table turns only with the wire clear of the foam: beside it, out past its turning circle,
when the next pass starts on the same side (alternating passes always do), or above it when the
wire has to cross over. After the last pass the part is cut free and the wire goes straight
home: up to the top of the travel, across to X0 and down when the part is in the way, else
straight to X0 Y0. The program starts from home too: straight to where the first cut begins, or
up over the stock and down when the part stands between home and that point; the wire comes on
once it is there. A program that leaves the machine's travel
says so at its top and in the status bar.

**HotwireCNC note:** its G-code interpreter currently ignores `B` words
(`crates/core/src/gcode/interp.rs`), so profile and slab programs at B = 0 run there today, but
programs that turn the table need B support added to that controller first.

## Layout

| Crate | What it holds | From CosmicDMS |
| --- | --- | --- |
| `crates/geom` | Vectors, regions and offsets, voxel stock, arc fitting | Copied |
| `crates/io` | Model readers, and `profile.rs`: 2D shape import | Copied, `profile.rs` new |
| `crates/render` | Camera, view cube, scene buffers | Copied |
| `crates/engine` | Session, commands, machine, stock, shapes, cuts, program, simulation, post | Command system copied; the hot-wire model new |
| `crates/ui-egui` | Ribbon, dock, panes, cut panel, viewport | Theme, ui_kit, ribbon, dock, icons, navigation, GPU viewport copied; panes adapted |
| `apps/CosmicCut` | The executable | Adapted |

## Test

    cargo test --workspace
    cargo clippy --workspace --all-targets

The `samples/` folder has a DXF arch and an SVG chess-pawn half profile to import as shapes,
and STL models to import: the sample penguin (made by `tools/penguin.py`) and an L-bracket.
# CosmicCut
