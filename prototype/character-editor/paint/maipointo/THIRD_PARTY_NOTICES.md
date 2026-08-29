# Third-party notices

## libmypaint (brushlib) — reference for this reimplementation

- Upstream: https://github.com/mypaint/libmypaint
- License: ISC
- Copyright (C) 2007-2014 Martin Renold and the MyPaint Development Team.
- Files used as the reference for this port:
  `vendor/libmypaint/mypaint-tiled-surface.c` (dab mask + LRE encoding),
  `vendor/libmypaint/brushmodes.c` (pixel blend modes),
  `vendor/libmypaint/mypaint-brush.c` (stroke state machine, pending),
  `vendor/libmypaint/mypaint-mapping.c` (input mappings, pending),
  `vendor/libmypaint/brushsettings.json` (settings/input tables, pending).

The ISC license permits modification and redistribution; this crate is an
independent Rust reimplementation of the above ISC-licensed math and carries
this notice and the upstream copyright notice per the license terms.
