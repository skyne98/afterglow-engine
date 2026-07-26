import {
  BigContainerAssetLoader,
  readBigHeader,
  type BigHeader,
  type FetchRangeLoader,
} from './big-parser.ts';

/** Parsed, indexed view of one cooked BIG container.
 *
 * This object owns no workers, renderer state, queues, or platform policy. It
 * only binds a byte source to immutable container metadata and raw-asset range
 * lookup. */
export class BigContainer {
  readonly rawAssets: BigContainerAssetLoader;

  private constructor(
    readonly source: FetchRangeLoader,
    readonly path: string,
    readonly header: BigHeader,
  ) {
    this.rawAssets = new BigContainerAssetLoader(source, path, header);
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
