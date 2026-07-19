/**
 * Font-size control for the static archive webapp
 * (spec: openspec/specs/static-archive-webapp/spec.md, Reader controls).
 *
 * An "Aa" trigger opening an A− / % (reset) / A+ stepper, stepping the
 * persisted scale in `fontScaleStorage`. The popover shape is ported from
 * Direction's `font-scale-control.tsx`; it opens *downward* here because this
 * control lives in a header rather than a sidebar footer.
 *
 * Desktop/WebUI builds never mount this — there `--app-font-scale` is owned
 * by `useAppInitialization`.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/utils";
import {
  MIN_TENTHS,
  MAX_TENTHS,
  DEFAULT_TENTHS,
  applyScale,
  loadStoredTenths,
  storeTenths,
} from "./fontScaleStorage";

/** The A− / % / A+ row. `%` resets; the ends disable at the range bounds. */
function Stepper({
  tenths,
  onStep,
  onReset,
}: {
  tenths: number;
  onStep: (delta: number) => void;
  onReset: () => void;
}) {
  const { t } = useTranslation();
  const stepClass =
    "min-h-6 rounded px-1.5 text-px12 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none disabled:opacity-40";

  return (
    <div
      role="group"
      aria-label={t("archive.web.fontSize")}
      className="flex items-center gap-0.5"
    >
      <button
        type="button"
        onClick={() => onStep(-1)}
        disabled={tenths <= MIN_TENTHS}
        aria-label={t("archive.web.fontSmaller")}
        title={t("archive.web.fontSmaller")}
        className={stepClass}
      >
        A−
      </button>
      <button
        type="button"
        onClick={onReset}
        disabled={tenths === DEFAULT_TENTHS}
        aria-label={t("archive.web.fontReset")}
        title={t("archive.web.fontReset")}
        className="min-h-6 w-11 rounded text-center text-px11 tabular-nums text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:pointer-events-none"
      >
        {tenths * 10}%
      </button>
      <button
        type="button"
        onClick={() => onStep(1)}
        disabled={tenths >= MAX_TENTHS}
        aria-label={t("archive.web.fontLarger")}
        title={t("archive.web.fontLarger")}
        className={stepClass}
      >
        A+
      </button>
    </div>
  );
}

export function FontScaleControl() {
  const { t } = useTranslation();
  const [tenths, setTenths] = useState<number>(() => loadStoredTenths());
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    applyScale(tenths);
    storeTenths(tenths);
  }, [tenths]);

  // Dismiss on outside pointer or Escape, matching Direction's popover.
  useEffect(() => {
    if (!open) return;
    const onPointerDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  const step = useCallback((delta: number) => {
    setTenths((prev) =>
      Math.min(MAX_TENTHS, Math.max(MIN_TENTHS, prev + delta))
    );
  }, []);

  const reset = useCallback(() => setTenths(DEFAULT_TENTHS), []);

  return (
    <div ref={rootRef} className="relative" data-testid="font-scale-control">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-label={t("archive.web.fontSize")}
        aria-expanded={open}
        title={t("archive.web.fontSize")}
        className={cn(
          "h-7 rounded-md px-2 text-px13 leading-none transition-colors hover:bg-muted hover:text-foreground",
          open ? "bg-muted text-foreground" : "text-muted-foreground"
        )}
      >
        Aa
      </button>
      {open && (
        <div className="absolute top-full right-0 z-50 mt-1 rounded-md border border-border bg-popover p-1 shadow-md">
          <Stepper tenths={tenths} onStep={step} onReset={reset} />
        </div>
      )}
    </div>
  );
}
