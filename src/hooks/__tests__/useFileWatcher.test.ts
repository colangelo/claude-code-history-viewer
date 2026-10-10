import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { renderHook, waitFor, act } from '@testing-library/react';

const { mockToastError } = vi.hoisted(() => ({
  mockToastError: vi.fn(),
}));

vi.mock('sonner', () => ({
  toast: {
    error: mockToastError,
  },
}));

/** Minimal EventSource stand-in: records instances and lets a test emit events. */
class FakeEventSource {
  static readonly CLOSED = 2;
  static instances: FakeEventSource[] = [];

  readonly url: string;
  readyState = 1;
  onerror: (() => void) | null = null;
  close = vi.fn(() => {
    this.readyState = FakeEventSource.CLOSED;
  });
  private listeners = new Map<string, (e: MessageEvent) => void>();

  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }

  addEventListener(name: string, cb: (e: MessageEvent) => void) {
    this.listeners.set(name, cb);
  }

  emit(name: string, data: string) {
    this.listeners.get(name)?.({ data } as MessageEvent);
  }
}

import { useFileWatcher } from '../useFileWatcher';

const eventPayload = {
  projectPath: '/test/project',
  sessionPath: '/test/session.jsonl',
  eventType: 'changed',
};

describe('useFileWatcher', () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    vi.stubGlobal('EventSource', FakeEventSource);
    localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetAllMocks();
  });

  describe('initial state', () => {
    it('does not connect when enabled is false', async () => {
      renderHook(() => useFileWatcher({ enabled: false }));
      await new Promise((resolve) => setTimeout(resolve, 10));
      expect(FakeEventSource.instances).toHaveLength(0);
    });

    it('opens an SSE connection to /api/events on mount', async () => {
      const { result } = renderHook(() => useFileWatcher());

      await waitFor(() => {
        expect(result.current.isWatching).toBe(true);
      });
      expect(FakeEventSource.instances).toHaveLength(1);
      expect(FakeEventSource.instances[0]?.url).toMatch(/\/api\/events$/);
    });

    it('passes the stored auth token as a query parameter', async () => {
      localStorage.setItem('webui-auth-token', 'tok en');
      renderHook(() => useFileWatcher());

      await waitFor(() => {
        expect(FakeEventSource.instances).toHaveLength(1);
      });
      expect(FakeEventSource.instances[0]?.url).toMatch(/\/api\/events\?token=tok%20en$/);
    });
  });

  describe('cleanup', () => {
    it('closes the connection on unmount', async () => {
      const { unmount } = renderHook(() => useFileWatcher());
      await waitFor(() => {
        expect(FakeEventSource.instances).toHaveLength(1);
      });

      unmount();

      expect(FakeEventSource.instances[0]?.close).toHaveBeenCalledTimes(1);
    });

    it('closes the connection and clears isWatching on stopWatching', async () => {
      const { result } = renderHook(() => useFileWatcher());
      await waitFor(() => {
        expect(result.current.isWatching).toBe(true);
      });

      act(() => {
        result.current.stopWatching();
      });

      expect(FakeEventSource.instances[0]?.close).toHaveBeenCalledTimes(1);
      expect(result.current.isWatching).toBe(false);
    });
  });

  describe('events', () => {
    it('calls onSessionChanged with the parsed payload', async () => {
      const onSessionChanged = vi.fn();
      renderHook(() => useFileWatcher({ onSessionChanged, debounceMs: 0 }));
      await waitFor(() => {
        expect(FakeEventSource.instances).toHaveLength(1);
      });

      FakeEventSource.instances[0]?.emit('session-file-changed', JSON.stringify(eventPayload));
      await new Promise((resolve) => setTimeout(resolve, 50));

      expect(onSessionChanged).toHaveBeenCalledWith(eventPayload);
    });

    it('ignores a payload that is not JSON', async () => {
      vi.spyOn(console, 'warn').mockImplementation(() => {});
      const onSessionChanged = vi.fn();
      renderHook(() => useFileWatcher({ onSessionChanged, debounceMs: 0 }));
      await waitFor(() => {
        expect(FakeEventSource.instances).toHaveLength(1);
      });

      FakeEventSource.instances[0]?.emit('session-file-changed', 'not json');
      await new Promise((resolve) => setTimeout(resolve, 50));

      expect(onSessionChanged).not.toHaveBeenCalled();
    });

    it('debounces rapid events with the same key', async () => {
      vi.useFakeTimers();
      const onSessionChanged = vi.fn();
      renderHook(() => useFileWatcher({ onSessionChanged, debounceMs: 300 }));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10);
      });

      const es = FakeEventSource.instances[0];
      act(() => {
        es?.emit('session-file-changed', JSON.stringify(eventPayload));
        es?.emit('session-file-changed', JSON.stringify(eventPayload));
        es?.emit('session-file-changed', JSON.stringify(eventPayload));
      });
      expect(onSessionChanged).not.toHaveBeenCalled();

      await act(async () => {
        await vi.advanceTimersByTimeAsync(350);
      });

      expect(onSessionChanged).toHaveBeenCalledTimes(1);
      expect(onSessionChanged).toHaveBeenCalledWith(eventPayload);
      vi.useRealTimers();
    });
  });

  describe('disconnection', () => {
    it('clears isWatching and toasts when the connection closes for good', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {});
      const { result } = renderHook(() => useFileWatcher());
      await waitFor(() => {
        expect(result.current.isWatching).toBe(true);
      });

      const es = FakeEventSource.instances[0];
      act(() => {
        if (es) es.readyState = FakeEventSource.CLOSED;
        es?.onerror?.();
      });

      expect(result.current.isWatching).toBe(false);
      expect(mockToastError).toHaveBeenCalledWith(
        'Live file watching disconnected. Refresh to reconnect.'
      );
    });

    it('stays watching on a transient error (EventSource reconnects itself)', async () => {
      const { result } = renderHook(() => useFileWatcher());
      await waitFor(() => {
        expect(result.current.isWatching).toBe(true);
      });

      act(() => {
        FakeEventSource.instances[0]?.onerror?.();
      });

      expect(result.current.isWatching).toBe(true);
      expect(mockToastError).not.toHaveBeenCalled();
    });
  });
});
