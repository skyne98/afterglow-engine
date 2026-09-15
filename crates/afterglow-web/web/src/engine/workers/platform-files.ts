type FileOps = {
  op_open_file(extensions: string[]): Promise<Uint8Array | null>;
  op_save_file(name: string, bytes: Uint8Array): Promise<boolean>;
};
function fileOps(): FileOps | undefined {
  const deno = (globalThis as typeof globalThis & { Deno?: { core?: { ops?: Partial<FileOps> } } }).Deno;
  if (!deno) return undefined;
  const ops = deno.core?.ops;
  if (!ops?.op_open_file || !ops.op_save_file) throw new Error('Native file dialogs are unavailable.');
  return ops as FileOps;
}

/** A dialog-selected file, with no filesystem path exposed to the caller. */
export async function openFile(extensions: string[]): Promise<Blob | null> {
  const ops = fileOps();
  if (ops) {
    const bytes = await ops.op_open_file(extensions);
    return bytes === null ? null : new Blob([bytes as BlobPart]);
  }
  return new Promise(resolve => {
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = extensions.map(extension => `.${extension}`).join(',');
    input.hidden = true;
    const complete = () => { const file = input.files?.[0] ?? null; input.remove(); resolve(file); };
    input.addEventListener('change', complete, { once: true });
    input.addEventListener('cancel', complete, { once: true });
    document.body.append(input);
    input.click();
  });
}

/** Native uses a save dialog. Public web uses a browser download. */
export async function saveFile(data: Uint8Array, name: string, type: string): Promise<boolean> {
  const ops = fileOps();
  if (ops) return ops.op_save_file(name, data);
  const url = URL.createObjectURL(new Blob([data as BlobPart], { type }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  return true;
}
