import type { SVGProps } from 'react';

type P = SVGProps<SVGSVGElement> & { size?: number };

function base(size: number, rest: SVGProps<SVGSVGElement>) {
  return {
    width: size,
    height: size,
    viewBox: '0 0 24 24',
    fill: 'none',
    stroke: 'currentColor',
    strokeWidth: 2,
    strokeLinecap: 'round' as const,
    strokeLinejoin: 'round' as const,
    'aria-hidden': true,
    ...rest,
  };
}

export const Logo = ({ size = 26 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 26 26" aria-hidden="true">
    <rect x="1" y="1" width="24" height="24" rx="7" fill="var(--accent)" />
    <path d="M7 9h12M7 13h7M7 17h10" stroke="var(--accent-ink)" strokeWidth="2.2" strokeLinecap="round" />
  </svg>
);

export const IconLive = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="M3 12h4l3-8 4 16 3-8h4" />
  </svg>
);
export const IconSearch = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </svg>
);
export const IconAlert = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="M12 3 2 20h20L12 3Z" />
    <path d="M12 10v4M12 17h.01" />
  </svg>
);
export const IconJourney = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <circle cx="5" cy="6" r="2" />
    <circle cx="19" cy="18" r="2" />
    <path d="M7 6h6a4 4 0 0 1 0 8H11a4 4 0 0 0 0 8h6" />
  </svg>
);
export const IconGrid = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <rect x="3" y="3" width="7" height="7" rx="1.5" />
    <rect x="14" y="3" width="7" height="7" rx="1.5" />
    <rect x="3" y="14" width="7" height="7" rx="1.5" />
    <rect x="14" y="14" width="7" height="7" rx="1.5" />
  </svg>
);
export const IconBell = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="M6 8a6 6 0 1 1 12 0c0 7 3 8 3 8H3s3-1 3-8" />
    <path d="M10 21h4" />
  </svg>
);
export const IconBookmark = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="M6 3h12v18l-6-4-6 4V3Z" />
  </svg>
);
export const IconSettings = ({ size = 16, ...r }: P) => (
  <svg {...base(size, r)}>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z" />
  </svg>
);
export const IconChevron = ({ size = 14, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="m6 9 6 6 6-6" />
  </svg>
);
export const IconClock = ({ size = 15, ...r }: P) => (
  <svg {...base(size, r)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7v5l3 2" />
  </svg>
);
export const IconPlay = ({ size = 14 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
    <path d="M7 4v16l13-8L7 4Z" />
  </svg>
);
export const IconCopy = ({ size = 15, ...r }: P) => (
  <svg {...base(size, r)}>
    <rect x="9" y="9" width="12" height="12" rx="2" />
    <path d="M5 15V5a2 2 0 0 1 2-2h10" />
  </svg>
);
export const IconReplay = ({ size = 15, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
    <path d="M3 3v5h5" />
  </svg>
);
export const IconLink = ({ size = 15, ...r }: P) => (
  <svg {...base(size, r)}>
    <path d="M10 14a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-1 1" />
    <path d="M14 10a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l1-1" />
  </svg>
);
export const IconMarker = ({ size = 9 }: { size?: number }) => (
  <svg width={size} height={size} viewBox="0 0 10 10" aria-hidden="true">
    <path d="M5 10 0 0h10L5 10Z" fill="var(--accent)" />
  </svg>
);
