import type { EngineTelemetry } from '../telemetry/telemetry.ts';
import { NativePersistentBlobBackend } from './native-persistent-blob-backend.ts';
import { WebPersistentBlobBackend } from './web-persistent-blob-backend.ts';
import {
  PersistentBlobStore,
  type PersistentBlobStoreCapacities,
} from './persistent-blob-store.ts';

type NativeDeno = { core?: { ops?: { op_afterglow_rpc_call_async?: unknown } } };

function hasNativeStorage(): boolean {
  return typeof (globalThis as typeof globalThis & { Deno?: NativeDeno })
    .Deno?.core?.ops?.op_afterglow_rpc_call_async === 'function';
}

/** Select native OS-worker files in afterglow-shell or OPFS on public web. */
export async function createPlatformPersistentBlobStore(
  namespace: string,
  capacities: Readonly<PersistentBlobStoreCapacities>,
  telemetry?: EngineTelemetry,
): Promise<PersistentBlobStore> {
  const backend = hasNativeStorage()
    ? NativePersistentBlobBackend.open(
        namespace, capacities.maxItems, capacities.maxValueBytes, telemetry,
      )
    : await WebPersistentBlobBackend.open(
        namespace, capacities.maxItems, capacities.maxValueBytes, telemetry,
      );
  return PersistentBlobStore.open(backend, capacities, telemetry);
}
