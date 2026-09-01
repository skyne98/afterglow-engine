export type PaintTileKey = [string, number, number, number];

const DB_NAME = 'afterglow-paint';
const DB_VERSION = 1;
const TILE_STORE = 'tiles';

export type PaintTileRecord = {
  key: PaintTileKey;
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
  return new Error(target?.error?.message ?? fallback);
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
      const range = IDBKeyRange.bound(
        [documentId, layer, tx0, ty0],
        [documentId, layer, tx1, ty1],
      );
      const request = tiles.openCursor(range);
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor) return;
        const record = cursor.value as TileRecord;
        if (record.ty >= ty0 && record.ty <= ty1 &&
            record.tx >= tx0 && record.tx <= tx1) {
          result.push({
            key: [record.documentId, record.layer, record.tx, record.ty],
            data: record.data,
          });
        }
        cursor.continue();
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB tile read failed.'));
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
      const tx = db.transaction(TILE_STORE, 'readwrite');
      tx.objectStore(TILE_STORE).clear();
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
      const tx = db.transaction(TILE_STORE, 'readwrite');
      const index = tx.objectStore(TILE_STORE).index('documentId');
      const request = index.openCursor(IDBKeyRange.only(documentId));
      request.onsuccess = () => {
        const cursor = request.result;
        if (cursor) {
          cursor.delete();
          cursor.continue();
        }
      };
      request.onerror = event => reject(errorText(event, 'IndexedDB document delete failed.'));
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
