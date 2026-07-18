import { clsx, type ClassValue } from "clsx"
import { extendTailwindMerge } from "tailwind-merge"

// The app defines fixed-pixel font sizes as custom utilities (`text-px11`…
// `text-px14`, see src/index.css, issue #408). tailwind-merge doesn't know
// these are font sizes, so by default it mistakes them for text-color classes
// and DROPS them whenever a real `text-<color>` class follows in the same
// `cn(...)` — which is exactly why tool-card headers silently inherited the
// 16px default. Teaching tailwind-merge that `text-pxNN` belongs to the
// font-size group keeps the size alongside the color and lets one `text-pxNN`
// correctly override another.
const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [
        { text: ["px6", "px8", "px9", "px10", "px11", "px12", "px13", "px14"] },
      ],
    },
  },
})

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
