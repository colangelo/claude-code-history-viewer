/**
 * Font-size control for the static archive webapp
 * (spec: openspec/specs/static-archive-webapp/spec.md, Reader controls).
 *
 * A− / % (reset) / A+ stepping the persisted scale in `fontScaleStorage`.
 * Desktop/WebUI builds never mount this — there `--app-font-scale` is owned
 * by `useAppInitialization`.
 */

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  MIN_TENTHS,
  MAX_TENTHS,
  DEFAULT_TENTHS,
  applyScale,
  loadStoredTenths,
  storeTenths,
} from "./fontScaleStorage";

export function FontScaleControl() {
  const { t } = useTranslation();
  const [tenths, setTenths] = useState<number>(() => loadStoredTenths());

  useEffect(() => {
    applyScale(tenths);
    storeTenths(tenths);
  }, [tenths]);

  const step = useCallback((delta: number) => {
    setTenths((prev) =>
      Math.min(MAX_TENTHS, Math.max(MIN_TENTHS, prev + delta))
    );
  }, []);

  const buttonClass =
    "h-7 w-7 rounded-md border border-border text-px12 hover:bg-muted disabled:opacity-40";

  return (
    <div className="flex items-center gap-1" data-testid="font-scale-control">
      <button
        type="button"
        onClick={() => step(-1)}
        disabled={tenths <= MIN_TENTHS}
        aria-label={t("archive.web.fontSmaller")}
        title={t("archive.web.fontSmaller")}
        className={buttonClass}
      >
        A−
      </button>
      <button
        type="button"
        onClick={() => setTenths(DEFAULT_TENTHS)}
        aria-label={t("archive.web.fontReset")}
        title={t("archive.web.fontReset")}
        className="h-7 min-w-11 rounded-md px-1 text-px12 tabular-nums text-muted-foreground hover:bg-muted"
      >
        {tenths * 10}%
      </button>
      <button
        type="button"
        onClick={() => step(1)}
        disabled={tenths >= MAX_TENTHS}
        aria-label={t("archive.web.fontLarger")}
        title={t("archive.web.fontLarger")}
        className={buttonClass}
      >
        A+
      </button>
    </div>
  );
}
