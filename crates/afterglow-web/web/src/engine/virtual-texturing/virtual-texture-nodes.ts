import type * as THREE_TYPES from 'three/webgpu';
import type * as TSL_TYPES from 'three/tsl';
import { PAGE_BORDER, PAGE_SIZE } from './virtual-texture-format.ts';
import { VT_FEEDBACK_WGSL, VT_SAMPLE_WGSL } from './virtual-texture-shaders.ts';
import type { VirtualTextureView } from './virtual-texture-system.ts';

type ThreeNodeRuntime = typeof THREE_TYPES & typeof TSL_TYPES;

export interface VirtualTextureSampleNodeOptions {
  readonly uv: THREE_TYPES.Node<'vec2'>;
  readonly mipBias?: number;
  readonly filter?: 'linear' | 'nearest';
}

export interface VirtualTextureFeedbackNodeOptions {
  readonly sampleUv?: THREE_TYPES.Node<'vec2'>;
  readonly gradientUv?: THREE_TYPES.Node<'vec2'>;
  readonly pixelScale: THREE_TYPES.Vector2;
  readonly qualityBias?: number;
}

/** Visible sample and matching feedback producer for one arbitrary shader input. */
export class VirtualTextureNodeBinding {
  /** Storage-domain texel before interpretation (for custom data shaders). */
  readonly rawValue: THREE_TYPES.Node<'vec4'>;
  /** Descriptor-interpreted value; sRGB pool values are converted to linear. */
  readonly value: THREE_TYPES.Node<'vec4'>;
  private readonly sampleUv: THREE_TYPES.Node<'vec2'>;

  constructor(
    private readonly three: ThreeNodeRuntime,
    readonly view: VirtualTextureView,
    options: Readonly<VirtualTextureSampleNodeOptions>,
  ) {
    this.sampleUv = options.uv;
    const entry = view.entry;
    const atlas = three.texture(view.store.atlasTexture);
    const sample = three.wgslFn(VT_SAMPLE_WGSL);
    const addressMode = view.descriptor.addressMode === 'repeat' ? 1
      : view.descriptor.addressMode === 'mirror-repeat' ? 2 : 0;
    this.rawValue = sample({
      pageTable: three.texture(entry.pageTableTexture),
      atlas,
      atlasSampler: three.sampler(atlas),
      uv: options.uv,
      virtualSize: three.uniform(new three.Vector2(entry.width, entry.height)),
      pageGrid: three.uniform(new three.Vector2(entry.pageGridX, entry.pageGridY)),
      pageSize: three.float(PAGE_SIZE),
      pageBorder: three.float(PAGE_BORDER),
      atlasSize: three.uniform(new three.Vector2(view.store.atlasWidth, view.store.atlasHeight)),
      maxMip: three.float(entry.maxMip),
      textureMaxMip: three.float(entry.textureMaxMip),
      mipBias: three.float(options.mipBias ?? 0),
      filterMode: three.uint(options.filter === 'nearest' ? 1 : 0),
      addressMode: three.uint(addressMode),
    }) as THREE_TYPES.Node<'vec4'>;
    this.value = view.descriptor.format.endsWith('-srgb')
      ? three.vec4(three.sRGBTransferEOTF(this.rawValue.rgb), this.rawValue.a)
      : this.rawValue;
  }

  feedback(options: Readonly<VirtualTextureFeedbackNodeOptions>): THREE_TYPES.Node<'uvec4'> {
    const entry = this.view.entry;
    const feedback = this.three.wgslFn(VT_FEEDBACK_WGSL);
    const addressMode = this.view.descriptor.addressMode === 'repeat' ? 1
      : this.view.descriptor.addressMode === 'mirror-repeat' ? 2 : 0;
    return feedback({
      sampleUV: options.sampleUv ?? this.sampleUv,
      gradientUV: options.gradientUv ?? this.sampleUv,
      feedbackPixelScale: this.three.uniform(options.pixelScale),
      virtualSize: this.three.uniform(new this.three.Vector2(entry.width, entry.height)),
      pageGrid: this.three.uniform(new this.three.Vector2(entry.pageGridX, entry.pageGridY)),
      maxMip: this.three.float(entry.maxMip),
      qualityBias: this.three.float(options.qualityBias ?? 0),
      addressMode: this.three.uint(addressMode),
      textureId: this.three.uint(entry.textureId),
      viewDistance: this.three.positionView.length(),
      cameraNear: this.three.cameraNear,
      cameraFar: this.three.cameraFar,
    }) as THREE_TYPES.Node<'uvec4'>;
  }
}

export function virtualTextureNode(
  three: ThreeNodeRuntime,
  view: VirtualTextureView,
  options: Readonly<VirtualTextureSampleNodeOptions>,
): VirtualTextureNodeBinding {
  return new VirtualTextureNodeBinding(three, view, options);
}
