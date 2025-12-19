// WASM TypeScript entry point
// Import WASM module dynamically after build
async function init(): Promise<void> {
  // Load generated WASM bindings and initialize module
  // Path is relative to where this JS file is served (dist/js/)
  // @ts-expect-error - Generated at build time
  const wasmModule = await import('../wasm/nu_analytics.js');

  // Initialize the WASM binary before calling wrappers
  await wasmModule.default();

  const { get_wasm_version, greet, sample_logs } = wasmModule as {
    get_wasm_version: () => string;
    greet: (name: string) => string;
    sample_logs: () => string;
  };

  const appDiv = document.getElementById('app');

  // Emit sample logs from the WASM side to demonstrate logging wiring
  const sampleLogResult = sample_logs();

  if (appDiv) {
    appDiv.innerHTML = `
      <h1>NuAnalytics</h1>
      <p>${get_wasm_version()}</p>
      <p>${greet('User')}</p>
      <p>${sampleLogResult}</p>
    `;
  }
}

init().catch((err) => {
  // eslint-disable-next-line no-console
  console.error('Failed to initialize WASM', err);
  const appDiv = document.getElementById('app');
  if (appDiv) {
    appDiv.innerHTML = '<p style="color:red">Failed to load NuAnalytics WASM.</p>';
  }
});
