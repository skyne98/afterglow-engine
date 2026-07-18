import { expect, test } from 'bun:test';
import type { RpcTransport } from './codec.ts';
import { PhysicsClient } from './physics.client.ts';

test('generated client closes an owned transport exactly once', () => {
  let terminations = 0;
  const transport: RpcTransport = {
    async call(): Promise<Uint8Array> { return new Uint8Array(); },
    terminate(): void { terminations++; },
  };
  const client = new PhysicsClient(transport);
  client.close();
  client.close();
  expect(terminations).toBe(1);
});
