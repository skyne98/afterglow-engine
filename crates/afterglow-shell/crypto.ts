// The host supplies cryptographic bytes from the operating system.
export function createCrypto(fill) {
  const typedArrayName = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(Uint8Array.prototype), Symbol.toStringTag,
  ).get;
  const integers = new Set([
    'Int8Array', 'Uint8Array', 'Uint8ClampedArray', 'Int16Array',
    'Uint16Array', 'Int32Array', 'Uint32Array', 'BigInt64Array', 'BigUint64Array',
  ]);
  return {
    getRandomValues(array) {
      if (!integers.has(typedArrayName.call(array))) {
        throw new DOMException('An integer typed array is necessary', 'TypeMismatchError');
      }
      if (array.byteLength > 65_536) {
        throw new DOMException('The random byte limit is 65536', 'QuotaExceededError');
      }
      fill(new Uint8Array(array.buffer, array.byteOffset, array.byteLength));
      return array;
    },
    randomUUID() {
      const bytes = new Uint8Array(16);
      fill(bytes);
      bytes[6] = (bytes[6] & 0x0f) | 0x40;
      bytes[8] = (bytes[8] & 0x3f) | 0x80;
      const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
      return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
    },
  };
}
