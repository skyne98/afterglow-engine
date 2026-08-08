import type { AssetByteRange } from './bulk-range.ts';
import { BigContainerAssetLoader } from './big-container-asset-loader.ts';
import { readBigHeader, type FetchRangeLoader } from './asset-range.ts';
import type { BigHeader } from './big-format.ts';

export interface ContainerRangeSource {
  read(offset: number, length: number): Promise<Uint8Array>;
  readBulk?: ((ranges: readonly AssetByteRange[]) => Promise<Uint8Array[]>) | undefined;
}

/** Parsed, indexed view of one cooked BIG container.
 *
 * This object owns no workers, renderer state, queues, or platform policy. It
 * only binds a byte source to immutable container metadata and raw-asset range
 * lookup. */
export class BigContainer {
  readonly rawAssets: BigContainerAssetLoader;
  readonly ranges: ContainerRangeSource;

  private constructor(
    readonly source: FetchRangeLoader,
    readonly path: string,
    readonly header: BigHeader,
  ) {
    this.rawAssets = new BigContainerAssetLoader(source, path, header);
    this.ranges = {
      read: (offset, length) => source.read(path, offset, length),
      readBulk: source.readBulk
        ? ranges => source.readBulk!(path, ranges)
        : undefined,
    };
  }

  static async open(
    source: FetchRangeLoader,
    path: string,
    maxHeaderBytes: number,
  ): Promise<BigContainer> {
    if (!path) throw new RangeError('BIG container path is required');
    const header = await readBigHeader(source, path, maxHeaderBytes);
    return new BigContainer(source, path, header);
  }
}
