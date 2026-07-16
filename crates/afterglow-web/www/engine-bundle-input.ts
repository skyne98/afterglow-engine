import * as THREE from 'three/webgpu';
import * as TSL from 'three/tsl';
import { createWorld, addEntity, addComponent, query } from 'bitecs';
import { VirtualTextureFeedbackPass } from './engine/virtual-texture-feedback-pass.ts';
import { EngineMemory, EnginePhase, FixedIndexPool, LinearArena, defineEngineMemoryResource } from './engine/engine-memory.ts';
import { BudgetDecision, FrameBudget, FrameBudgetRes, FrameStage } from './engine/frame-budget.ts';
import {
  VirtualTextureStore, VT_SAMPLE_WGSL, VT_SAMPLE_LEVEL_WGSL,
  VT_RESOLVE_MATERIAL_MIP4_WGSL, VT_FEEDBACK_WGSL,
  FORMAT_RGBA, PAGE_SIZE, PAGE_BORDER, SLOT_SIZE,
  ATLAS_WIDTH, ATLAS_HEIGHT,
} from './engine/virtual-texture.ts';

window.THREE = THREE;
Object.assign(window.THREE, TSL);
window.bitecsCreateWorld = createWorld;
window.bitecsAddEntity = addEntity;
window.bitecsAddComponent = addComponent;
window.bitecsQuery = query;
window.AfterglowMemory = {
  EngineMemory, EnginePhase, FixedIndexPool, LinearArena, defineEngineMemoryResource,
  BudgetDecision, FrameBudget, FrameBudgetRes, FrameStage,
};
window.AfterglowVT = {
  VirtualTextureStore, VirtualTextureFeedbackPass, VT_SAMPLE_WGSL,
  VT_SAMPLE_LEVEL_WGSL, VT_RESOLVE_MATERIAL_MIP4_WGSL, VT_FEEDBACK_WGSL,
  FORMAT_RGBA, PAGE_SIZE, PAGE_BORDER, SLOT_SIZE, ATLAS_WIDTH, ATLAS_HEIGHT,
};
