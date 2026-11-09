import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

const domain = readFileSync(new URL('./domain.txt', import.meta.url), 'utf8').trim();
const REPO = 'https://github.com/karaa1122/trailscope';

const docs = (...slugs) => slugs.map((s) => `docs/${s}`);

export default defineConfig({
  site: `https://${domain}`,
  trailingSlash: 'always',
  integrations: [
    starlight({
      title: 'Trailscope',
      description:
        'Self-hosted API request logging: see every request your services handle, query them with SQL and follow one customer across services and jobs.',
      logo: { src: './src/assets/logo.svg', replacesTitle: false },
      favicon: '/favicon.svg',
      social: [{ icon: 'github', label: 'GitHub', href: REPO }],
      customCss: ['./src/styles/fonts.css', './src/styles/starlight.css'],
      components: {
        SocialIcons: './src/components/DocsNav.astro',
        Footer: './src/components/DocsFooter.astro',
      },
      head: [
        { tag: 'meta', attrs: { property: 'og:image', content: `https://${domain}/og.png` } },
        { tag: 'meta', attrs: { name: 'twitter:card', content: 'summary_large_image' } },
      ],
      credits: false,
      sidebar: [
        {
          label: 'Start here',
          items: docs('', 'self-hosting', 'configuration', 'security').map((s) =>
            s.replace(/\/$/, ''),
          ),
        },
        {
          label: 'Send data',
          items: docs(
            'sdk-express',
            'sdk-fastify',
            'sdk-nestjs',
            'sdk-node',
            'sdk-python',
            'sdk-java',
            'sdk-laravel',
            'ingest-protocol',
            'sdk-spec',
          ),
        },
        { label: 'Use Trailscope', items: docs('query-language', 'alerts', 'api') },
        { label: 'Internals', items: docs('architecture', 'benchmarks') },
      ],
    }),
  ],
  vite: {
    // The landing page imports the screenshots from ../docs/images and the
    // fonts from ../web.
    server: { fs: { allow: ['..'] } },
  },
});
