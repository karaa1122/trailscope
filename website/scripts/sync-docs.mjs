// Builds the documentation pages from ../docs, which stays the single source:
// the same Markdown reads well on GitHub and on the site.
//
// For each published page it
// - turns the first "# Title" into Starlight frontmatter (title, sidebar label,
//   description, edit link),
// - rewrites relative links: other doc pages become /docs/<page>/, anything
//   else in the repository (examples, DECISIONS.md, SECURITY.md) a GitHub link,
// - turns ```mermaid blocks into <pre class="mermaid"> (rendered in the browser),
// and fails on a link to a page that doesn't exist.
//
// It also copies the logo and the social preview into public/.

import { copyFile, mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const REPO = 'https://github.com/karaa1122/trailscope';
const repoRoot = new URL('../../', import.meta.url);
const docsDir = new URL('docs/', repoRoot);
const outDir = new URL('../src/content/docs/docs/', import.meta.url);
const publicDir = new URL('../public/', import.meta.url);

/** Pages kept off the site (still linked, to GitHub). */
const UNPUBLISHED = new Set(['DECISIONS.md']);

/** Short sidebar labels; pages not listed use their title. */
const LABELS = {
  README: 'Introduction',
  'sdk-spec': 'SDK behaviour',
  'sdk-node': 'Node.js core',
  'sdk-express': 'Express',
  'sdk-fastify': 'Fastify',
  'sdk-nestjs': 'NestJS',
  'sdk-python': 'Python, Django, FastAPI',
  'sdk-java': 'Java and Spring Boot',
  'sdk-laravel': 'PHP and Laravel',
  'ingest-protocol': 'Ingest protocol and OTLP',
};

/** Fence languages the highlighter doesn't know. */
const FENCE_ALIASES = { ebnf: 'text' };

const files = (await readdir(docsDir)).filter((f) => f.endsWith('.md')).sort();
const published = new Set(files.filter((f) => !UNPUBLISHED.has(f)));

function slugOf(file) {
  return file === 'README.md' ? '' : file.replace(/\.md$/, '');
}

function pageUrl(file) {
  const slug = slugOf(file);
  return slug ? `/docs/${slug}/` : '/docs/';
}

function rewriteLink(target, file) {
  if (/^([a-z]+:|#|\/)/i.test(target)) return target;
  const [path, hash = ''] = target.split(/(?=#)/);
  const anchor = hash;
  if (path.startsWith('../')) {
    const repoPath = path.slice(3).replace(/\/$/, '');
    const kind = /\.[a-z0-9]+$/i.test(repoPath) ? 'blob' : 'tree';
    return `${REPO}/${kind}/main/${repoPath}${anchor}`;
  }
  if (path.endsWith('.md')) {
    if (published.has(path)) return pageUrl(path) + anchor;
    if (files.includes(path)) return `${REPO}/blob/main/docs/${path}${anchor}`;
    throw new Error(`${file}: link to a missing page: ${target}`);
  }
  if (path.startsWith('images/')) return `${REPO}/raw/main/docs/${path}`;
  throw new Error(`${file}: don't know how to publish the link ${target}`);
}

function escapeHtml(s) {
  return s.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
}

function plain(markdown) {
  return markdown
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/[`*_]/g, '')
    .replace(/\s+/g, ' ')
    .trim();
}

function convert(file, source) {
  const lines = source.replace(/\r\n/g, '\n').split('\n');
  const first = lines.findIndex((l) => l.trim() !== '');
  const heading = /^#\s+(.+)$/.exec(lines[first] ?? '');
  if (!heading) throw new Error(`${file}: the first line must be a "# Title"`);
  const title = plain(heading[1]);
  const body = lines.slice(first + 1);

  const out = [];
  let fence = null; // { marker, lang, lines }
  let description = '';
  for (const line of body) {
    const open = /^(\s*)(```+|~~~+)\s*([\w+-]*)/.exec(line);
    if (fence) {
      if (open && open[2].startsWith(fence.marker) && !open[3]) {
        if (fence.lang === 'mermaid') {
          out.push(`<pre class="mermaid">${escapeHtml(fence.lines.join('\n'))}</pre>`);
        } else {
          out.push(fence.open, ...fence.lines, line);
        }
        fence = null;
      } else {
        fence.lines.push(line);
      }
      continue;
    }
    if (open) {
      const lang = open[3];
      const alias = FENCE_ALIASES[lang];
      fence = {
        marker: open[2],
        lang,
        open: alias ? line.replace(lang, alias) : line,
        lines: [],
      };
      continue;
    }
    if (!description && /^[A-Za-z`*[]/.test(line) && !line.startsWith('|')) {
      description = plain(line);
    }
    out.push(
      line.replace(/(\]\()([^)\s]+)(\))/g, (_, a, target, b) => a + rewriteLink(target, file) + b),
    );
  }
  if (fence) throw new Error(`${file}: unclosed code fence`);

  if (description.length > 180) description = `${description.slice(0, 177).trimEnd()}…`;
  const slug = slugOf(file) || 'README';
  const front = [
    '---',
    `title: ${JSON.stringify(title)}`,
    description ? `description: ${JSON.stringify(description)}` : null,
    `editUrl: ${JSON.stringify(`${REPO}/edit/main/docs/${file}`)}`,
    LABELS[slug] ? `sidebar:\n  label: ${JSON.stringify(LABELS[slug])}` : null,
    '---',
    '',
    `<!-- Generated from docs/${file} by website/scripts/sync-docs.mjs. Edit that file. -->`,
  ].filter((l) => l !== null);
  return `${front.join('\n')}\n${out.join('\n').replace(/^\n+/, '\n')}`;
}

await rm(outDir, { recursive: true, force: true });
await mkdir(outDir, { recursive: true });
for (const file of published) {
  const source = await readFile(new URL(file, docsDir), 'utf8');
  const slug = slugOf(file) || 'index';
  await writeFile(new URL(`${slug}.md`, outDir), convert(file, source));
}

await mkdir(publicDir, { recursive: true });
await copyFile(new URL('docs/images/logo.svg', repoRoot), new URL('favicon.svg', publicDir));
await copyFile(new URL('docs/images/social-preview.png', repoRoot), new URL('og.png', publicDir));

console.log(`sync-docs: ${published.size} pages -> ${fileURLToPath(outDir)}`);
