# TrenchBroom: Tool & Entity Map

> A structured breakdown of TrenchBroom's tools, user interactions, and the entity/brush model.

## Core Units

### Map Structure (TrenchBroom's View)

```
World = {Property} DefaultLayer {Layer}
Layer = Name {Group} {Entity} {Brush}
Group = Name {Group} {Entity} {Brush}
```

| Unit | Description |
|---|---|
| **World** | Root of the hierarchy, holds global properties (worldspawn). |
| **Layer** | Named partition of the map. Hides/shows groups of objects. |
| **Group** | Named hierarchy of entities + brushes. Can be nested arbitrarily. Linked groups update across instances. |
| **Entity** | Sequence of properties + zero or more brushes. Two kinds: *point entities* (no brushes, have a position/origin) and *brush entities* (one or more brushes). |
| **Brush** | Convex polyhedron defined by intersecting half-spaces of its face planes. |
| **Face** | Single plane + material (texture) + UV attributes (offset, scale, rotation). |
| **Property** | Key-value string pair on an entity. |
| **Brush Geometry** | Computed vertices, edges, facets from the plane representation (same algorithm as BSP compilers). |

### Entity Types

| Type | Description |
|---|---|
| **Point Entity** | A single position in space (e.g., `info_player_start`, `light`, `monster_zombie`). No brushes. Has a model shown in the editor. |
| **Brush Entity** | Entity with one or more brushes (e.g., `func_door`, `func_platform`). Brushes define its collision shape. |

## Tools

Tools are divided into **permanently active** (always available unless a modal tool is active) and **modal** (manually activated/deactivated).

### Tool Table

| Tool | Type | Purpose |
|---|---|---|
| **Camera / Look** | Permanent | RMB drag to look. Scroll to move forward/back. MMB drag to pan/up-down. |
| **Select / Move** | Permanent | LMB click to select objects. Drag to move. Arrow keys to nudge. |
| **Extrude** | Modal | Drag a brush face to extend/shrink it. Ctrl + drag face to move it without changing adjacent faces. |
| **Clip** | Modal | Place 2–3 clip points to define a cutting plane. Splits brush into two, or chops off one side. |
| **Vertex** | Modal | Select and move individual vertices, edges, or faces of brushes. Supports multi-select and edge/face splitting. Can create new vertices by splitting edges. |
| **Rotate** | Modal | Rotate selected objects about a configurable center. |
| **Scale** | Modal | Uniform or non-uniform scaling of selected objects about a center. |
| **Shear** | Modal | Shear transformation of selected objects. |
| **CSG Merge** | Operation | Convex merge of selected brushes (replaces union — only works if result is convex). |
| **CSG Subtract** | Operation | Subtract one brush from another. Creates multiple convex brushes to represent the concave result. |
| **CSG Hollow** | Operation | Hollows out a brush by creating a shell with a configurable wall thickness. |
| **CSG Intersect** | Operation | Intersection of overlapping brushes. |
| **Drill** | Modal | Special tool for 2D viewports. Left-click drag to cut along 2D lines. |
| **Make Cuboid** | Permanent | Click-drag to create an axis-aligned box brush in 3D or 2D views. |

### Permanent Tool Details

| Interaction | How |
|---|---|
| **Select object** | LMB click |
| **Multi-select** | Shift + LMB |
| **Move object** | Drag with LMB on selected object |
| **Nudge** | Arrow keys (10 units default, Shift = 1 unit) |
| **Duplicate** | Ctrl + D |
| **Delete** | Delete / Backspace |
| **Undo / Redo** | Ctrl+Z / Ctrl+Y |

### Clip Tool

1. Activate (toolbar or keyboard shortcut)
2. Click in any viewport to place clip points (2 points = infinite cut plane, 3 points = finite triangle cut)
3. Preview shows which side will be kept (yellow) and which will be removed (red)
4. Press Enter/Space to perform the cut, or flip the clip direction
5. The brush is split into two brushes along the clip plane

### Vertex Tool

- Select individual **vertices**, **edges**, or **faces** of a brush
- Drag to move them — TrenchBroom recomputes the brush from the modified geometry
- **Edge splitting**: select an edge midpoint → creates a new vertex → allows more complex shapes
- **Face splitting**: select a face → inserts a new vertex on the face
- Multi-select vertices with Shift and move them together
- TrenchBroom validates the brush after each operation and rejects invalid geometry
- The plane-based brush representation means vertex edits are translated back to plane changes

### CSG Operations

| Operation | Input | Output |
|---|---|---|
| **Convex Merge** | Multiple overlapping brushes | One convex brush (fails if result would be concave) |
| **Subtract** | One brush subtracted from another | Multiple convex brushes representing the difference |
| **Hollow** | Single brush | Shell of brushes with configurable wall thickness |
| **Intersect** | Multiple overlapping brushes | One convex brush representing intersection volume |

## Camera Navigation

| Action | Input |
|---|---|
| Look around | RMB drag in 3D view |
| Move forward/back | Scroll wheel |
| Move towards cursor | Scroll wheel (with pref option) |
| Pan / up-down | MMB drag |
| Orbit | Ctrl + RMB drag (click point = orbit center) |
| Move forward/back (keyboard) | W / S |
| Strafe left/right | A / D |
| Move up/down | Q / E |
| Center on selection | F |
| Move camera to position | Ctrl + Shift + F |

## 2D Viewport Navigation

| Action | Input |
|---|---|
| Pan | MMB or RMB drag |
| Zoom | Scroll wheel |
| Zoom linked across 2D views | Yes (zoom + pan along shared axes) |
| Viewport types | XY (top), XZ (front), YZ (side) |

## Entity System

### Property Editing

| Interaction | How |
|---|---|
| **Entity Inspector** | Right panel, shows all key-value properties of selected entity |
| **Add property** | Type key + value in inspector |
| **Smart editors** | Type-specific widgets for colors, angles, choices (spawnflags), targets, models |
| **Entity Browser** | Drag-and-drop from a list of defined entities to create them |
| **Entity link visualization** | Dashed lines between linked entities (e.g., `target` / `targetname`) |
| **3D model display** | Renders MDL, MD2, MD3, BSP, DKM models in the viewport |

### Entity Definitions (FGD / ENT / DEF)

Entity definitions tell TrenchBroom:
- The entity's classname and description
- Which properties it has (name, type, default value, choices)
- spawnflags bit definitions
- Which model to display in the viewport
- Color for rendering in the editor

### Mod System

- Mods are subdirectories with custom assets
- Priority-based resolution: higher priority mod wins name conflicts
- Default mod (e.g., `id1` for Quake) always lowest priority
- Each mod can provide its own entity definition file and models

## Face / Material System

| Concept | Description |
|---|---|
| **Material** | Defines surface rendering. Closely tied to textures. |
| **Material Collection** | Directory of loose images, or a WAD archive. |
| **UV Editor** | Visual editor in the Face Inspector for adjusting texture alignment. |
| **Face attributes** | Texture name, X/Y offset, X/Y scale, rotation angle. |
| **Valve 220 format** | Adds texture lock and UV skewing support. |
| **Precision texture lock** | All brush operations preserve UV coordinates. |

## Layers

| Interaction | How |
|---|---|
| **Create layer** | Map Inspector → Layers |
| **Move objects to layer** | Select → right-click → Move to Layer |
| **Hide/show layer** | Toggle visibility in the Map Inspector |
| **Layer locking** | Prevent edits on locked layers |

## Selection & Rendering

| State | Visual |
|---|---|
| **Selected object** | Red edges + tinted faces; bounding box with spikes at corners |
| **Hovered object** | Spikes from bounding box corners |
| **Selected face** | Highlighted face |
| **Brush geometry** | Calculated from plane representation on-the-fly |
| **Compass** | Bottom-left: RGB = XYZ |
| **Grid** | Configurable size, visible in 2D viewports |

## Issue Browser

- Lists problems with the map (invalid brushes, missing textures, etc.)
- Provides "auto-fix" buttons for common issues
- Runs continuously in the info bar at the bottom

## External Pipeline

```
TrenchBroom → .MAP file → BSP compiler → .BSP → Game engine
                         → Light compiler → .LIT
                         → Vis compiler  → .VIS
```

TrenchBroom can launch external compilers and the game engine directly from the menu.

## References

- [TrenchBroom Manual](https://trenchbroom.github.io/manual/latest/)
- [Level Design Book — TrenchBroom](https://book.leveldesignbook.com/appendix/tools/trenchbroom)
- [Valve Developer Union — TrenchBroom](https://valvedev.info/tools/trenchbroom/)
