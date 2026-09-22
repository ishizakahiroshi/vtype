// Test harness (not a test file): chrome.storage with both areas and change events.

import type { StorageChangeListener, StorageView } from "../src/shared/settings";

export interface Stub {
  readonly view: StorageView;
  readonly sync: Record<string, unknown>;
  readonly local: Record<string, unknown>;
}

export function stubStorage(
  initial: { sync?: Record<string, unknown>; local?: Record<string, unknown> } = {},
  broken = false,
): Stub {
  const sync: Record<string, unknown> = structuredClone(initial.sync ?? {});
  const local: Record<string, unknown> = structuredClone(initial.local ?? {});
  const listeners: StorageChangeListener[] = [];
  const refuse = (): never => {
    throw new Error("storage is unavailable");
  };
  const areaView = (items: Record<string, unknown>, area: string) => ({
    get: async (keys: string | string[] | null) => {
      if (broken) refuse();
      const wanted = keys === null ? Object.keys(items) : typeof keys === "string" ? [keys] : keys;
      const out: Record<string, unknown> = {};
      for (const key of wanted) if (key in items) out[key] = structuredClone(items[key]);
      return out;
    },
    set: async (next: Record<string, unknown>) => {
      if (broken) refuse();
      const changes: Record<string, { newValue?: unknown }> = {};
      for (const [key, value] of Object.entries(next)) {
        items[key] = structuredClone(value);
        changes[key] = { newValue: structuredClone(value) };
      }
      for (const l of [...listeners]) l(changes, area);
    },
  });
  return {
    view: {
      sync: areaView(sync, "sync"),
      local: areaView(local, "local"),
      onChanged: {
        addListener: (l) => listeners.push(l),
        removeListener: (l) => {
          const i = listeners.indexOf(l);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    },
    sync,
    local,
  };
}

/** Let the pending storage promises run. */
export async function settle(times = 8): Promise<void> {
  for (let i = 0; i < times; i++) await Promise.resolve();
}
