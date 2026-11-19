/** Classes every anchored popup shares: surface, border, shadow and motion. */
export const popupSurface =
	"z-50 rounded-md border border-border bg-popover text-popover-foreground shadow-[0_8px_24px_-6px_rgb(0_0_0/0.18),0_1px_2px_rgb(0_0_0/0.06)] outline-none";

export const popupMotion =
	"origin-(--transform-origin) transition-[opacity,scale] duration-100 data-starting-style:scale-[0.97] data-starting-style:opacity-0 data-ending-style:scale-[0.97] data-ending-style:opacity-0 motion-reduce:transition-none";

/** Classes for one row in a menu or a select list. */
export const popupItem =
	"flex h-7 cursor-default items-center gap-2 rounded px-2 text-[13px] whitespace-nowrap outline-none select-none data-highlighted:bg-accent data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:size-3.5 [&_svg]:shrink-0 [&_svg]:text-muted-foreground";
