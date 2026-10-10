import { beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "zustand";
import {
  createServerSlice,
  type ServerSlice,
} from "@/store/slices/serverSlice";
import { api } from "@/services/api";

vi.mock("@/services/api", () => ({
  api: vi.fn(),
}));

const createTestStore = () =>
  create<ServerSlice>()((set, get) => ({
    ...createServerSlice(
      set as unknown as Parameters<typeof createServerSlice>[0],
      get as unknown as Parameters<typeof createServerSlice>[1],
      {} as unknown as Parameters<typeof createServerSlice>[2],
    ),
  }));

describe("serverSlice", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("loads read-only mode from WebUI server config", async () => {
    vi.mocked(api).mockResolvedValue({ readOnly: true });
    const useStore = createTestStore();

    await useStore.getState().loadServerConfig();

    expect(api).toHaveBeenCalledWith("get_server_config");
    expect(useStore.getState().isServerReadOnly).toBe(true);
    expect(useStore.getState().isServerConfigLoaded).toBe(true);
  });

  it("falls back to writable when the server config cannot be read", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => {});
    vi.mocked(api).mockRejectedValue(new Error("offline"));
    const useStore = createTestStore();

    await useStore.getState().loadServerConfig();

    expect(useStore.getState().isServerReadOnly).toBe(false);
    expect(useStore.getState().isServerConfigLoaded).toBe(true);
  });
});
