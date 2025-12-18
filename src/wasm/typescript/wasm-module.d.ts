// Type declaration shim to make the generated wasm-bindgen module discoverable to TS
declare module '../wasm/nu_analytics.js' {
  export * from '../../dist/wasm/nu_analytics';
  const init: typeof import('../../dist/wasm/nu_analytics.js').default;
  export default init;
}
