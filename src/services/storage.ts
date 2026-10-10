/**
 * Storage adapter — `localStorage` with a per-store namespace prefix.
 *
 * Usage:
 *   import { storageAdapter } from "@/services/storage";
 *   const store = await storageAdapter.load("settings.json");
 *   await store.set("key", value);
 *   const val = await store.get("key");
 */

export interface StoreHandle {
  get<T = unknown>(key: string): Promise<T | null>;
  set(key: string, value: unknown): Promise<void>;
  save(): Promise<void>;
}

/**
 * Load (or create) a named store, backed by `localStorage` under `webui:<name>:`.
 */
async function loadStore(
  name: string,
  _options?: { defaults?: Record<string, unknown>; autoSave?: boolean },
): Promise<StoreHandle> {
  const prefix = `webui:${name}:`;
  const defaults = _options?.defaults;
  return {
    get: <T = unknown>(key: string) => {
      try {
        const raw = localStorage.getItem(`${prefix}${key}`);
        if (raw != null) return Promise.resolve(JSON.parse(raw) as T);
        // Fall back to the caller's defaults for keys never written
        if (defaults && key in defaults) return Promise.resolve(defaults[key] as T);
        return Promise.resolve(null);
      } catch {
        return Promise.resolve(null);
      }
    },
    set: (key: string, value: unknown) => {
      try {
        localStorage.setItem(`${prefix}${key}`, JSON.stringify(value));
      } catch {
        // localStorage full or unavailable — silently ignore
      }
      return Promise.resolve();
    },
    save: () => Promise.resolve(), // localStorage writes are immediate
  };
}

export const storageAdapter = { load: loadStore };
