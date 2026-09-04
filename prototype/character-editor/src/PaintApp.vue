<script setup lang="ts">
import {
  Brush,
  ChevronDown,
  ChevronUp,
  FileDown,
  FilePlus2,
  FlipHorizontal2,
  FolderOpen,
  FolderPlus,
  Hand,
  ImageDown,
  Layers3,
  Palette,
  PanelRightClose,
  Pipette,
  Plus,
  Redo2,
  RotateCcw,
  RotateCw,
  Save,
  Search,
  Trash2,
  Undo2,
  ZoomIn,
  ZoomOut,
} from '@lucide/vue';
import { Button } from '@/components/ui/button';
import ColorPanel from './ColorPanel.vue';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu';
import { Separator } from '@/components/ui/separator';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip';

const layerModes = ['Normal', 'Multiply', 'Screen', 'Overlay', 'Darken', 'Lighten', 'Hard Light', 'Soft Light', 'Burn', 'Dodge', 'Difference', 'Exclusion', 'Hue', 'Saturation', 'Color', 'Luminosity', 'Plus', 'Destination In', 'Destination Out', 'Source Atop', 'Destination Atop', 'Pigment'];

function activate(id: string): void {
  document.getElementById(id)?.click();
}

function focusColor(): void {
  document.getElementById('colorHex')?.focus();
}
</script>

<template>
  <div class="paint-app">
    <header class="menu-bar">
      <div class="brand" aria-label="Afterglow Paint">A</div>
      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <Button variant="ghost" size="sm">File</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" class="w-56">
          <DropdownMenuItem @select="activate('newDocumentBtn')">
            <FilePlus2 /> New
            <DropdownMenuShortcut>Ctrl+N</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem @select="activate('importOraInput')">
            <FolderOpen /> Open OpenRaster
            <DropdownMenuShortcut>Ctrl+O</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem @select="activate('exportOraBtn')">
            <Save /> Save OpenRaster
            <DropdownMenuShortcut>Ctrl+S</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem @select="activate('exportPngBtn')">
            <ImageDown /> Export PNG
            <DropdownMenuShortcut>Ctrl+Alt+Shift+W</DropdownMenuShortcut>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <Button variant="ghost" size="sm">Edit</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" class="w-48">
          <DropdownMenuItem @select="activate('undoBtn')">
            <Undo2 /> Undo
            <DropdownMenuShortcut>Ctrl+Z</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuItem @select="activate('redoBtn')">
            <Redo2 /> Redo
            <DropdownMenuShortcut>Ctrl+Shift+Z</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" @select="activate('clearBtn')">
            <Trash2 /> Clear layer
            <DropdownMenuShortcut>Delete</DropdownMenuShortcut>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <DropdownMenu>
        <DropdownMenuTrigger as-child>
          <Button variant="ghost" size="sm">View</Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" class="w-52">
          <DropdownMenuItem @select="activate('fitViewBtn')">Fit on screen <DropdownMenuShortcut>Ctrl+0</DropdownMenuShortcut></DropdownMenuItem>
          <DropdownMenuItem @select="activate('actualPixelsBtn')">100% <DropdownMenuShortcut>Ctrl+1</DropdownMenuShortcut></DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem @select="activate('rotateLeftBtn')"><RotateCcw /> Rotate left</DropdownMenuItem>
          <DropdownMenuItem @select="activate('rotateRightBtn')"><RotateCw /> Rotate right</DropdownMenuItem>
          <DropdownMenuItem @select="activate('mirrorBtn')"><FlipHorizontal2 /> Flip canvas</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem @select="activate('panelToggleBtn')">Hide panels <DropdownMenuShortcut>Tab</DropdownMenuShortcut></DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <span class="document-name">Untitled-1 @ <span id="zoomVal">100</span>%</span>
    </header>

    <div class="options-bar">
      <span class="option-title">Brush</span>
      <label>Size <input id="radius" type="range" min="2" max="60" value="14" /><output id="radiusVal" /></label>
      <label>Hardness <input id="hardness" type="range" min="0" max="1" step="0.01" value="0.6" /><output id="hardnessVal" /></label>
      <label>Opacity <input id="opacity" type="range" min="0" max="1" step="0.01" value="1" /><output id="opacityVal" /></label>
      <button id="colorFocusBtn" class="color-control" type="button" @click="focusColor">Color <span class="toolbar-color-swatch" /></button>
    </div>

    <div class="workspace">
      <TooltipProvider :delay-duration="350">
        <aside class="tool-rail" aria-label="Tools">
          <Tooltip>
            <TooltipTrigger as-child><Button id="brushToolBtn" class="tool-active" variant="ghost" size="icon" aria-label="Brush tool"><Brush /></Button></TooltipTrigger>
            <TooltipContent side="right">Brush tool (B)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child><Button id="eyedropperToolBtn" variant="ghost" size="icon" aria-label="Eyedropper tool"><Pipette /></Button></TooltipTrigger>
            <TooltipContent side="right">Eyedropper tool (I or hold Alt)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child><Button id="handToolBtn" variant="ghost" size="icon" aria-label="Hand tool"><Hand /></Button></TooltipTrigger>
            <TooltipContent side="right">Hand tool (H or Space)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child><Button id="zoomToolBtn" variant="ghost" size="icon" aria-label="Zoom tool"><Search /></Button></TooltipTrigger>
            <TooltipContent side="right">Zoom tool (Z)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child><Button id="rotateToolBtn" variant="ghost" size="icon" aria-label="Rotate view tool"><RotateCw /></Button></TooltipTrigger>
            <TooltipContent side="right">Rotate view tool (R)</TooltipContent>
          </Tooltip>
          <Separator class="my-1" />
          <Tooltip>
            <TooltipTrigger as-child><Button id="zoomOutBtn" variant="ghost" size="icon" aria-label="Zoom out"><ZoomOut /></Button></TooltipTrigger>
            <TooltipContent side="right">Zoom out (Ctrl+-)</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child><Button id="zoomInBtn" variant="ghost" size="icon" aria-label="Zoom in"><ZoomIn /></Button></TooltipTrigger>
            <TooltipContent side="right">Zoom in (Ctrl++)</TooltipContent>
          </Tooltip>
          <button class="swatches" type="button" title="Default colors (D), switch colors (X)" aria-label="Foreground and background colors" @click="focusColor">
            <span class="foreground-swatch" />
            <span class="background-swatch" />
          </button>
        </aside>
      </TooltipProvider>

      <main id="viewport">
        <canvas id="paint" width="900" height="600" tabindex="0" aria-label="Paint canvas" />
        <div id="hud" hidden />
      </main>

      <aside id="panels" class="panel-dock" aria-label="Panels">
        <section class="panel color-panel">
          <h2><Palette /> Color</h2>
          <ColorPanel />
        </section>

        <section class="panel brush-panel">
          <h2><Brush /> Brushes</h2>
          <div id="brushGrid" aria-label="MyPaint brush presets">Loading brushes…</div>
        </section>

        <section class="panel layers-panel">
          <h2><Layers3 /> Layers</h2>
          <div class="layer-properties">
            <select id="layerBlendMode" aria-label="Blend mode">
              <option v-for="(mode, index) in layerModes" :key="mode" :value="index">{{ mode }}</option>
            </select>
            <label>Opacity <input id="layerOpacity" type="range" min="0" max="1" step="0.01" value="1" /><output id="layerOpacityValue">100%</output></label>
            <label>Parent <select id="layerParent" aria-label="Parent group"><option value="-1">Canvas</option></select></label>
            <div id="groupOptions" class="group-options" hidden>
              <label><input id="groupPassThrough" type="checkbox" /> Pass through</label>
              <label><input id="groupIsolated" type="checkbox" /> Isolate</label>
            </div>
          </div>
          <div id="layerList" class="layer-list" aria-label="Layer stack" />
          <div class="layer-actions" aria-label="Layer actions">
            <Button id="addLayerBtn" variant="ghost" size="icon-sm" title="New layer" aria-label="New layer"><Plus /></Button>
            <Button id="addGroupBtn" variant="ghost" size="icon-sm" title="New group" aria-label="New group"><FolderPlus /></Button>
            <span class="action-spacer" />
            <Button id="moveSelectionUpBtn" variant="ghost" size="icon-sm" title="Move up" aria-label="Move up"><ChevronUp /></Button>
            <Button id="moveSelectionDownBtn" variant="ghost" size="icon-sm" title="Move down" aria-label="Move down"><ChevronDown /></Button>
            <Button id="deleteSelectionBtn" variant="ghost" size="icon-sm" title="Delete" aria-label="Delete selected item"><Trash2 /></Button>
          </div>
        </section>

        <details class="panel">
          <summary>View</summary>
          <label class="field">Zoom <input id="viewZoom" type="range" min="0.1" max="8" step="0.01" value="1" /></label>
          <div class="button-grid">
            <Button id="rotateLeftBtn" variant="outline" size="sm"><RotateCcw /> Left</Button>
            <Button id="rotateRightBtn" variant="outline" size="sm"><RotateCw /> Right</Button>
            <Button id="mirrorBtn" variant="outline" size="sm"><FlipHorizontal2 /> Flip</Button>
            <Button id="resetViewBtn" variant="outline" size="sm">Reset</Button>
            <Button id="fitViewBtn" variant="outline" size="sm">Fit</Button>
            <Button id="actualPixelsBtn" variant="outline" size="sm">100%</Button>
          </div>
          <label class="check"><input id="frameEnabled" type="checkbox" /> Show canvas edge</label>
        </details>

        <details class="panel">
          <summary>Document</summary>
          <div class="number-grid">
            <label>Width <input id="documentWidth" type="number" min="64" max="16384" step="64" value="2048" /></label>
            <label>Height <input id="documentHeight" type="number" min="64" max="16384" step="64" value="2048" /></label>
          </div>
          <label class="field">Paint memory <span><output id="memoryLimitVal">1024</output> MiB</span><input id="memoryLimit" type="range" min="64" max="2048" step="64" value="1024" /></label>
          <label class="field inline-field">Background <input id="backgroundColor" type="color" value="#a8a498" /></label>
          <Button id="newDocumentBtn" variant="outline" size="sm"><FilePlus2 /> New document</Button>
          <input id="importOraInput" type="file" accept=".ora,image/openraster" hidden />
          <div class="button-grid">
            <Button id="exportOraBtn" variant="outline" size="sm"><FileDown /> Save ORA</Button>
            <Button id="exportPngBtn" variant="outline" size="sm"><ImageDown /> PNG</Button>
          </div>
        </details>

        <details class="panel diagnostics-panel">
          <summary>Diagnostics</summary>
          <div class="button-grid">
            <Button id="undoBtn" variant="outline" size="sm"><Undo2 /> Undo</Button>
            <Button id="redoBtn" variant="outline" size="sm"><Redo2 /> Redo</Button>
            <Button id="clearBtn" variant="outline" size="sm">Clear layer</Button>
            <Button id="strokeBtn" variant="outline" size="sm">Test stroke</Button>
          </div>
          <pre id="logs">…</pre>
        </details>
      </aside>
    </div>

    <footer class="status-bar">
      <span id="status">Loading brush engine…</span>
      <button id="panelToggleBtn" type="button"><PanelRightClose /> Panels</button>
    </footer>
  </div>
</template>
