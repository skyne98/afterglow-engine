export type PaintTileKey = [string, number, number, number];
export type PaintHistoryKey = [string, number, 0 | 1, number, number, number];
export type PaintEvictionRank = readonly [
  layer: number,
  index: number,
  tx: number,
  ty: number,
  behind: number,
  projection: number,
  distanceSquared: number,
];

export function paintEvictionRankBefore(
  left: PaintEvictionRank,
  right: PaintEvictionRank,
): boolean {
  if (left[4] !== right[4]) return left[4] > right[4];
  if (left[5] !== right[5]) return left[5] < right[5];
  if (left[6] !== right[6]) return left[6] > right[6];
  if (left[0] !== right[0]) return left[0] < right[0];
  if (left[2] !== right[2]) return left[2] < right[2];
  return left[3] < right[3];
}

export function protectedTileRegionContains(
  region: readonly [number, number, number, number],
  tx: number,
  ty: number,
  tilesWidth: number,
  tilesHeight: number,
  guardTiles: number,
): boolean {
  const tx0 = region[0] === 0 ? 0 : region[0] + guardTiles;
  const ty0 = region[1] === 0 ? 0 : region[1] + guardTiles;
  const tx1 = region[2] === tilesWidth - 1 ? region[2] : region[2] - guardTiles;
  const ty1 = region[3] === tilesHeight - 1 ? region[3] : region[3] - guardTiles;
  return tx >= tx0 && tx <= tx1 && ty >= ty0 && ty <= ty1;
}

const DB_NAME = 'afterglow-paint';
const DB_VERSION = 2;
const TILE_STORE = 'tiles';
const HISTORY_STORE = 'history';
export const PAINT_TILE_WRITE_BATCH_BYTES = 2 * 1024 * 1024;

export type PaintTileRecord = {
  key: PaintTileKey;
  data: ArrayBuffer;
};

export type PaintHistoryRecord = {
  key: PaintHistoryKey;
  data: ArrayBuffer;
};

export function paintTileColumnBounds(
  documentId: string,
  layer: number,
  tx: number,
  ty0: number,
  ty1: number,
): [PaintTileKey, PaintTileKey] {
  return [
    [documentId, layer, tx, ty0],
    [documentId, layer, tx, ty1],
  ];
}

export function paintTileWriteBatchBytes(records: readonly { data: ArrayBuffer }[]): number {
  let bytes = 0;
  for (const record of records) bytes += record.data.byteLength;
  return bytes;
}

type HistoryRecord = {
  documentId: string;
  operation: number;
  side: 0 | 1;
  layer: number;
  tx: number;
  ty: number;
  data: ArrayBuffer;
};

type TileRecord = {
  documentId: string;
  layer: number;
  tx: number;
  ty: number;
  data: ArrayBuffer;
};

function errorText(value: Event | IDBRequest | IDBTransaction | null, fallback: string): Error {
  const target = value instanceof Event ? value.target as IDBRequest | IDBTransaction | null : value;
  const error = new Error(target?.error?.message ?? fallback);
  if (target?.error?.name) error.name = target.error.name;
  return error;
}

/** Bounded tile byte storage for the paint worker's cold tile path. */
export class PaintTileStore {
  private db: IDBDatabase | null = null;

  async open(): Promise<void> {
    if (this.db) return;
    if (typeof indexedDB === 'undefined') {
      throw new Error('IndexedDB is not available.');
    }
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      const tiles = db.objectStoreNames.contains(TILE_STORE)
        ? request.transaction!.objectStore(TILE_STORE)
        : db.createObjectStore(TILE_STORE, {
          keyPath: ['documentId', 'layer', 'tx', 'ty'],
        });
      if (!tiles.indexNames.contains('documentId')) {
        tiles.createIndex('documentId', 'documentId', { unique: false });
      }
      const history = db.objectStoreNames.contains(HISTORY_STORE)
        ? request.transaction!.objectStore(HISTORY_STORE)
        : db.createObjectStore(HISTORY_STORE, {
          keyPath: ['documentId', 'operation', 'side', 'layer', 'tx', 'ty'],
        });
      if (!history.indexNames.contains('documentId')) {
        history.createIndex('documentId', 'documentId', { unique: false });
      }
    };
    this.db = await new Promise<IDBDatabase>((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(errorText(request, 'IndexedDB open failed.'));
      request.onblocked = () => reject(new Error('IndexedDB open is blocked.'));
    });
  }

  async put(key: PaintTileKey, data: ArrayBuffer): Promise<void> {
    const db = this.db;
    if (!db) throw new Error('IndexedDB is not open.');
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(TILE_STORE, 'readwrite');
      tx.objectStore(TILE_STORE).put({
        documentId: key[0], layer: key[1], tx: key[2], ty: key[3], data,
      } satisfies TileRecord);
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB tile write failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB tile write aborted.'));
    });
  }

  async putMany(records: readonly PaintTileRecord[]): Promise<void> {
    if (paintTileWriteBatchBytes(records) > PAINT_TILE_WRITE_BATCH_BYTES) {
      throw new Error('IndexedDB tile write batch exceeds 2 MiB.');
    }
    const db = this.db;
    if (!db) throw new Error('IndexedDB is not open.');
    if (records.length === 0) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(TILE_STORE, 'readwrite');
      const tiles = tx.objectStore(TILE_STORE);
      for (const record of records) {
        const key = record.key;
        tiles.put({
          documentId: key[0], layer: key[1], tx: key[2], ty: key[3], data: record.data,
        } satisfies TileRecord);
      }
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB tile write failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB tile write aborted.'));
    });
  }

  async get(key: PaintTileKey): Promise<PaintTileRecord | null> {
    const records = await this.getMany([key]);
    return records[0];
  }

  async getMany(keys: readonly PaintTileKey[]): Promise<(PaintTileRecord | null)[]> {
    const db = this.db;
    if (!db) throw new Error('IndexedDB is not open.');
    const result: (PaintTileRecord | null)[] = new Array(keys.length).fill(null);
    if (keys.length === 0) return result;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(TILE_STORE, 'readonly');
      const tiles = tx.objectStore(TILE_STORE);
      for (let index = 0; index < keys.length; index++) {
        const request = tiles.get(keys[index]);
        request.onsuccess = () => {
          const record = request.result as TileRecord | undefined;
          if (record) {
            result[index] = {
              key: [record.documentId, record.layer, record.tx, record.ty],
              data: record.data,
            };
          }
        };
        request.onerror = event => reject(errorText(event, 'IndexedDB tile read failed.'));
      }
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB tile read failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB tile read aborted.'));
    });
    return result;
  }

  async putHistoryMany(records: readonly PaintHistoryRecord[]): Promise<void> {
    if (paintTileWriteBatchBytes(records) > PAINT_TILE_WRITE_BATCH_BYTES) {
      throw new Error('IndexedDB history write batch exceeds 2 MiB.');
    }
    const db = this.db;
    if (!db) throw new Error('IndexedDB is not open.');
    if (records.length === 0) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(HISTORY_STORE, 'readwrite');
      const history = tx.objectStore(HISTORY_STORE);
      for (const record of records) {
        const key = record.key;
        history.put({
          documentId: key[0], operation: key[1], side: key[2],
          layer: key[3], tx: key[4], ty: key[5], data: record.data,
        } satisfies HistoryRecord);
      }
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB history write failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB history write aborted.'));
    });
  }

  async getHistoryBatch(
    documentId: string,
    operation: number,
    side: 0 | 1,
    after: PaintHistoryKey | null,
    limit = 64,
  ): Promise<PaintHistoryRecord[]> {
    const db = this.db;
    if (!db) throw new Error('IndexedDB is not open.');
    const result: PaintHistoryRecord[] = [];
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(HISTORY_STORE, 'readonly');
      const history = tx.objectStore(HISTORY_STORE);
      const lower: IDBValidKey = after ?? [documentId, operation, side];
      const upper: IDBValidKey = [documentId, operation, side, []];
      const request = history.openCursor(IDBKeyRange.bound(lower, upper, after !== null, false));
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor || result.length >= limit) return;
        const record = cursor.value as HistoryRecord;
        result.push({
          key: [record.documentId, record.operation, record.side, record.layer, record.tx, record.ty],
          data: record.data,
        });
        if (result.length < limit) cursor.continue();
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB history read failed.'));
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB history read failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB history read aborted.'));
    });
    return result;
  }

  async deleteHistoryDocument(documentId: string): Promise<void> {
    const db = this.db;
    if (!db) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(HISTORY_STORE, 'readwrite');
      const request = tx.objectStore(HISTORY_STORE).index('documentId')
        .openCursor(IDBKeyRange.only(documentId));
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor) return;
        cursor.delete();
        cursor.continue();
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB history delete failed.'));
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB history delete failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB history delete aborted.'));
    });
  }

  async deleteHistorySide(documentId: string, operation: number, side?: 0 | 1): Promise<void> {
    const db = this.db;
    if (!db) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(HISTORY_STORE, 'readwrite');
      const history = tx.objectStore(HISTORY_STORE);
      const lower: IDBValidKey = side === undefined
        ? [documentId, operation]
        : [documentId, operation, side];
      const upper: IDBValidKey = side === undefined
        ? [documentId, operation, []]
        : [documentId, operation, side, []];
      const request = history.openCursor(IDBKeyRange.bound(lower, upper));
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor) return;
        cursor.delete();
        cursor.continue();
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB history delete failed.'));
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB history delete failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB history delete aborted.'));
    });
  }

  async getRegion(
    documentId: string,
    layer: number,
    tx0: number,
    ty0: number,
    tx1: number,
    ty1: number,
  ): Promise<PaintTileRecord[]> {
    const db = this.db;
    if (!db) throw new Error('IndexedDB is not open.');
    const result: PaintTileRecord[] = [];
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(TILE_STORE, 'readonly');
      const tiles = tx.objectStore(TILE_STORE);
      for (let column = tx0; column <= tx1; column++) {
        const [lower, upper] = paintTileColumnBounds(documentId, layer, column, ty0, ty1);
        const request = tiles.openCursor(IDBKeyRange.bound(lower, upper));
        request.onsuccess = () => {
          const cursor = request.result;
          if (!cursor) return;
          const record = cursor.value as TileRecord;
          result.push({
            key: [record.documentId, record.layer, record.tx, record.ty],
            data: record.data,
          });
          cursor.continue();
        };
        request.onerror = event => reject(errorText(event, 'IndexedDB tile read failed.'));
      }
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB tile read failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB tile read aborted.'));
    });
    return result;
  }

  async clear(): Promise<void> {
    const db = this.db;
    if (!db) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction([TILE_STORE, HISTORY_STORE], 'readwrite');
      tx.objectStore(TILE_STORE).clear();
      tx.objectStore(HISTORY_STORE).clear();
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB clear failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB clear aborted.'));
    });
  }

  async dropLayer(documentId: string, layer: number): Promise<void> {
    const db = this.db;
    if (!db) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(TILE_STORE, 'readwrite');
      const index = tx.objectStore(TILE_STORE).index('documentId');
      const request = index.openCursor(IDBKeyRange.only(documentId));
      request.onsuccess = () => {
        const cursor = request.result;
        if (cursor) {
          const record = cursor.value as TileRecord;
          if (record.layer === layer) cursor.delete();
          cursor.continue();
        }
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB layer delete failed.'));
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB layer delete failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB layer delete aborted.'));
    });
  }

  async deleteLayer(documentId: string, layer: number): Promise<void> {
    const db = this.db;
    if (!db) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(TILE_STORE, 'readwrite');
      const tiles = tx.objectStore(TILE_STORE);
      const index = tiles.index('documentId');
      const request = index.openCursor(IDBKeyRange.only(documentId));
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor) return;
        const record = cursor.value as TileRecord;
        if (record.layer === layer) {
          cursor.delete();
        } else if (record.layer > layer) {
          cursor.delete();
          tiles.put({ ...record, layer: record.layer - 1 });
        }
        cursor.continue();
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB layer delete failed.'));
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB layer delete failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB layer delete aborted.'));
    });
  }

  async dropDocument(documentId: string): Promise<void> {
    const db = this.db;
    if (!db) return;
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction([TILE_STORE, HISTORY_STORE], 'readwrite');
      for (const storeName of [TILE_STORE, HISTORY_STORE]) {
        const request = tx.objectStore(storeName).index('documentId')
          .openCursor(IDBKeyRange.only(documentId));
        request.onsuccess = () => {
          const cursor = request.result;
          if (cursor) {
            cursor.delete();
            cursor.continue();
          }
        };
        request.onerror = event => reject(errorText(event, 'IndexedDB document delete failed.'));
      }
      tx.oncomplete = () => resolve();
      tx.onerror = event => reject(errorText(event, 'IndexedDB document delete failed.'));
      tx.onabort = event => reject(errorText(event, 'IndexedDB document delete aborted.'));
    });
  }

  close(): void {
    this.db?.close();
    this.db = null;
  }
}

export function paintTileKey(
  documentId: string,
  layer: number,
  tx: number,
  ty: number,
): PaintTileKey {
  return [documentId, layer, tx, ty];
}
