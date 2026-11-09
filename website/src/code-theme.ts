// Syntax colours for the landing page's code samples, from the product palette
// (web/src/styles/tokens.css): lime strings, blue functions, amber types.
export const codeTheme = {
  name: 'trailscope',
  type: 'dark' as const,
  colors: {
    'editor.background': '#111419',
    'editor.foreground': '#d2d6dc',
  },
  tokenColors: [
    {
      scope: ['comment', 'punctuation.definition.comment'],
      settings: { foreground: '#6b7586', fontStyle: 'italic' },
    },
    {
      scope: ['string', 'string.quoted', 'string.template', 'string.unquoted'],
      settings: { foreground: '#c8f65d' },
    },
    {
      scope: [
        'keyword',
        'storage',
        'storage.type',
        'storage.modifier',
        'keyword.control',
        'keyword.operator.new',
        'keyword.operator.expression',
      ],
      settings: { foreground: '#ff8a7d' },
    },
    {
      scope: ['entity.name.function', 'support.function', 'meta.function-call.generic'],
      settings: { foreground: '#8ab4f8' },
    },
    {
      scope: [
        'entity.name.type',
        'entity.name.class',
        'support.class',
        'support.type',
        'entity.name.tag',
        'meta.decorator',
        'punctuation.decorator',
      ],
      settings: { foreground: '#f5b544' },
    },
    {
      scope: ['constant', 'constant.numeric', 'constant.language', 'variable.other.constant'],
      settings: { foreground: '#f5b544' },
    },
    {
      scope: [
        'entity.other.attribute-name',
        'variable.other.property',
        'meta.object-literal.key',
        'support.type.property-name',
        'variable.other.key',
        'entity.name.key',
        'keyword.other.definition',
      ],
      settings: { foreground: '#b7bdc7' },
    },
    { scope: ['variable', 'variable.other.readwrite'], settings: { foreground: '#e7eaee' } },
    {
      scope: ['punctuation', 'meta.brace', 'keyword.operator'],
      settings: { foreground: '#9099a8' },
    },
  ],
};
