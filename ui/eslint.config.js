import js from '@eslint/js';
import globals from 'globals';
import svelte from 'eslint-plugin-svelte';
import tseslint from 'typescript-eslint';

const SINK_PROPS = ['innerHTML', 'outerHTML', 'insertAdjacentHTML', 'setHTMLUnsafe', 'createContextualFragment'];
const sinkSelectors = SINK_PROPS.flatMap((name) => [
  { selector: `MemberExpression[property.name='${name}']`, message: `${name} is banned (untrusted text stays text).` },
  { selector: `MemberExpression[property.value='${name}']`, message: `${name} is banned (untrusted text stays text).` },
]);

const restricted = [
  ...sinkSelectors,
  { selector: "MemberExpression[object.name='document'][property.name='write']", message: 'document.write is banned.' },
  { selector: "MemberExpression[object.name='document'][property.name='writeln']", message: 'document.writeln is banned.' },
  // Viewing never acks (system/05): no code path may reference the ack route.
  { selector: 'Literal[value=/\\/ack/i]', message: 'The dashboard must never call the ack route.' },
  { selector: 'TemplateElement[value.raw=/\\/ack/i]', message: 'The dashboard must never call the ack route.' },
];

export default tseslint.config(
  { ignores: ['dist', 'node_modules', 'test-results', 'playwright-report', 'e2e/screenshots', 'src/lib/types'] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...svelte.configs['flat/recommended'],
  {
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
    rules: {
      'no-eval': 'error',
      'no-implied-eval': 'error',
      'no-new-func': 'error',
      'no-script-url': 'error',
      'no-restricted-syntax': ['error', ...restricted],
      'no-restricted-properties': [
        'error',
        { object: 'window', property: 'eval', message: 'eval is banned.' },
        { object: 'globalThis', property: 'eval', message: 'eval is banned.' },
      ],
    },
  },
  {
    files: ['**/*.svelte', '**/*.svelte.ts'],
    languageOptions: { parserOptions: { parser: tseslint.parser, extraFileExtensions: ['.svelte'] } },
    rules: {
      'svelte/no-at-html-tags': 'error',
      'svelte/no-inline-styles': 'error',
      'svelte/no-target-blank': 'error',
    },
  },
  {
    // Test files build hostile strings and probe for sinks on purpose.
    files: ['**/*.test.ts', 'e2e/**/*.ts', 'src/test-setup.ts', 'eslint.config.js'],
    rules: { 'no-restricted-syntax': 'off' },
  },
);
