import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['test/typescript/**/*.spec.ts'],
    environment: 'node',
  },
});
