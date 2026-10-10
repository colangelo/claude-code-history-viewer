/**
 * Browser file dialogs: Blob downloads for saving, <input type="file"> for opening.
 */

interface SaveDialogOptions {
  filters?: { name: string; extensions: string[] }[];
  defaultPath?: string;
  mimeType?: string;
}

interface OpenDialogOptions {
  filters?: { name: string; extensions: string[] }[];
  multiple?: boolean;
  directory?: boolean;
  title?: string;
}

/**
 * Show a "Save file" dialog and write content.
 *
 * Triggers a browser download with the given content. Always returns `true`:
 * the browser owns the save prompt and does not report a cancel.
 */
export async function saveFileDialog(
  content: string,
  options?: SaveDialogOptions,
): Promise<boolean> {
  const filename = options?.defaultPath ?? "download.json";
  const blob = new Blob([content], { type: options?.mimeType ?? "application/json" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
  return true;
}

/**
 * Show a "Save file" dialog and write binary content (e.g., PNG image).
 *
 * Triggers a browser download with the given bytes. Returns `false` only if
 * building the download failed.
 */
export async function saveBinaryFileDialog(
  data: Uint8Array,
  options?: SaveDialogOptions & { mimeType?: string },
): Promise<boolean> {
  try {
    const filename = options?.defaultPath ?? "download.png";
    const mimeType = options?.mimeType ?? "image/png";
    const blob = new Blob([data], { type: mimeType });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    return true;
  } catch {
    return false;
  }
}

/**
 * Show an "Open file" dialog and read a single text file.
 *
 * Shows the browser file picker and reads the file with FileReader.
 *
 * Returns the file content string, or `null` if cancelled.
 */
export async function openFileDialog(
  options?: OpenDialogOptions,
): Promise<string | null> {
  return new Promise<string | null>((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    const exts = options?.filters?.flatMap((f) => f.extensions.map((e) => `.${e}`));
    if (exts?.length) {
      input.accept = exts.join(",");
    }
    let resolved = false;
    const safeResolve = (val: string | null) => {
      if (!resolved) {
        resolved = true;
        window.removeEventListener("focus", focusHandler);
        resolve(val);
      }
    };

    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) {
        safeResolve(null);
        return;
      }
      const reader = new FileReader();
      reader.onload = () => safeResolve(reader.result as string);
      reader.onerror = () => safeResolve(null);
      reader.readAsText(file);
    };
    // User cancelled — oncancel (Chrome 113+, Firefox 124+, Safari 16.4+)
    input.oncancel = () => safeResolve(null);
    // Fallback for older browsers: detect cancel via window focus
    const focusHandler = () => {
      setTimeout(() => {
        if (!input.files?.length) safeResolve(null);
      }, 500);
    };
    window.addEventListener("focus", focusHandler, { once: true });
    input.click();
  });
}
