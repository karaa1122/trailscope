import domainFile from '../domain.txt?raw';

/** The site's domain: website/domain.txt is the only place it is set. */
export const DOMAIN = domainFile.trim();
export const REPO = 'https://github.com/karaa1122/trailscope';
export const CONTACT = `hello@${DOMAIN}`;

export const NAV = [
  { href: '/#features', label: 'Features' },
  { href: '/docs/', label: 'Docs' },
  { href: '/pricing/', label: 'Pricing' },
] as const;
