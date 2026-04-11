function IconBase({
  children,
  className = "",
  size = "1em",
  title = "",
  viewBox = "0 0 24 24",
  strokeWidth = 1.8,
}) {
  const accessibleProps = title
    ? { role: "img", "aria-label": title }
    : { "aria-hidden": "true" };

  return (
    <svg
      {...accessibleProps}
      className={className}
      width={size}
      height={size}
      viewBox={viewBox}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      focusable="false"
      style={{ display: "block", flexShrink: 0 }}
    >
      {title ? <title>{title}</title> : null}
      {children}
    </svg>
  );
}

export function AppIcon({ name, className = "", size = "1em", title = "" }) {
  switch (name) {
    case "overview":
      return (
        <IconBase className={className} size={size} title={title}>
          <rect x="4" y="4" width="6" height="6" rx="1.5" />
          <rect x="14" y="4" width="6" height="6" rx="1.5" />
          <rect x="4" y="14" width="6" height="6" rx="1.5" />
          <rect x="14" y="14" width="6" height="6" rx="1.5" />
        </IconBase>
      );
    case "memory":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M8 4h8l4 8-4 8H8l-4-8Z" />
          <circle cx="12" cy="12" r="2.5" />
        </IconBase>
      );
    case "analytics":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M4 19h16" />
          <path d="M6 15.5 10 11l3 2.5 5-6" />
          <circle cx="6" cy="15.5" r="1" />
          <circle cx="10" cy="11" r="1" />
          <circle cx="13" cy="13.5" r="1" />
          <circle cx="18" cy="7.5" r="1" />
        </IconBase>
      );
    case "agents":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="8" cy="9" r="2.5" />
          <circle cx="16.5" cy="8" r="2" />
          <path d="M4.5 18c.9-2.4 2.9-3.5 5.5-3.5s4.6 1.1 5.5 3.5" />
          <path d="M14.3 18c.6-1.6 2-2.4 3.7-2.4 1 0 1.8.2 2.5.7" />
        </IconBase>
      );
    case "work":
    case "tasks":
      return (
        <IconBase className={className} size={size} title={title}>
          <rect x="6" y="5" width="12" height="15" rx="2" />
          <path d="M9 5.5h6" />
          <path d="m9.5 12.5 1.8 1.8 3.5-3.8" />
        </IconBase>
      );
    case "feed":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="7" cy="17" r="1.2" />
          <path d="M6 7.5a10 10 0 0 1 10.5 10.5" />
          <path d="M6 11.5a6 6 0 0 1 6.5 6.5" />
          <path d="M6 15a2.5 2.5 0 0 1 2.8 2.8" />
        </IconBase>
      );
    case "messages":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M6 7h12a2 2 0 0 1 2 2v6a2 2 0 0 1-2 2H10l-4 3v-3H6a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2Z" />
        </IconBase>
      );
    case "activity":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M3 12h4l2.2-4 4.1 8 2.4-4H21" />
        </IconBase>
      );
    case "locks":
      return (
        <IconBase className={className} size={size} title={title}>
          <rect x="6" y="11" width="12" height="9" rx="2" />
          <path d="M8.5 11V8.5a3.5 3.5 0 1 1 7 0V11" />
        </IconBase>
      );
    case "brain":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="7" cy="8" r="2" />
          <circle cx="17" cy="7" r="2" />
          <circle cx="10" cy="16" r="2" />
          <circle cx="18" cy="16" r="2" />
          <path d="M8.7 9.3 9.9 14.2" />
          <path d="M15.3 8.3 11.7 14.7" />
          <path d="m12 16 4 0" />
        </IconBase>
      );
    case "conflicts":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="m13 3-7 10h5l-1 8 8-11h-5l1-7Z" />
        </IconBase>
      );
    case "about":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="12" cy="12" r="8" />
          <path d="M12 10v5" />
          <circle cx="12" cy="7.2" r=".8" fill="currentColor" stroke="none" />
        </IconBase>
      );
    case "decision":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="7" cy="7" r="2" />
          <circle cx="17" cy="7" r="2" />
          <circle cx="12" cy="17" r="2" />
          <path d="M8.8 8.1 11 15" />
          <path d="M15.2 8.1 13 15" />
        </IconBase>
      );
    case "event":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="12" cy="12" r="7" />
          <path d="M12 8v4l3 2" />
        </IconBase>
      );
    case "token":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M6 8h12" />
          <path d="M4.5 12h15" />
          <path d="M7 16h10" />
        </IconBase>
      );
    case "savings":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M12 5v10" />
          <path d="m8.5 11.5 3.5 3.5 3.5-3.5" />
          <path d="M6 19h12" />
        </IconBase>
      );
    case "efficiency":
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="12" cy="12" r="7" />
          <circle cx="12" cy="12" r="3" />
        </IconBase>
      );
    case "refresh":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M19 11a7 7 0 0 0-12-4.5" />
          <path d="M5 7V3h4" />
          <path d="M5 13a7 7 0 0 0 12 4.5" />
          <path d="M19 17v4h-4" />
        </IconBase>
      );
    case "outbound":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="M7 17 17 7" />
          <path d="M9 7h8v8" />
        </IconBase>
      );
    case "chevron-left":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="m14.5 6-6 6 6 6" />
        </IconBase>
      );
    case "chevron-right":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="m9.5 6 6 6-6 6" />
        </IconBase>
      );
    case "close":
      return (
        <IconBase className={className} size={size} title={title}>
          <path d="m7 7 10 10" />
          <path d="M17 7 7 17" />
        </IconBase>
      );
    default:
      return (
        <IconBase className={className} size={size} title={title}>
          <circle cx="12" cy="12" r="8" />
          <path d="M12 8v5" />
          <circle cx="12" cy="17" r=".8" fill="currentColor" stroke="none" />
        </IconBase>
      );
  }
}
