import domainFile from '../domain.txt?raw';

/** The site's domain: website/domain.txt is the only place it is set. */
export const DOMAIN = domainFile.trim();
export const REPO = 'https://github.com/karaa1122/trailscope';
export const IMAGE = 'ghcr.io/karaa1122/trailscope';
export const CONTACT = `hello@${DOMAIN}`;

/** Where the SDKs are published, shown under the code tabs on the landing page. */
export const REGISTRIES = [
  { name: 'npm', pkg: '@trailscope/*', href: 'https://www.npmjs.com/org/trailscope' },
  { name: 'PyPI', pkg: 'trailscope', href: 'https://pypi.org/project/trailscope/' },
  {
    name: 'Packagist',
    pkg: 'trailscope/trailscope',
    href: 'https://packagist.org/packages/trailscope/trailscope',
  },
  {
    name: 'Maven Central',
    pkg: 'pro.trailscope',
    href: 'https://central.sonatype.com/namespace/pro.trailscope',
  },
] as const;

export const NAV = [
  { href: '/#features', label: 'Features' },
  { href: '/docs/', label: 'Docs' },
  { href: '/pricing/', label: 'Pricing' },
] as const;
