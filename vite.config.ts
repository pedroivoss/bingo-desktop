import { defineConfig } from "vite";
// @ts-expect-error type error without @types/node package
import process from "node:process";
import obfuscatorPkg from "rollup-plugin-obfuscator";
// pacote CommonJS: a função vem em default (às vezes default.default)
const obfuscator: any = (obfuscatorPkg as any).default ?? obfuscatorPkg;

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(({ command }) => ({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  // Produção: código das telas minificado e OFUSCADO (dificulta ler e alterar o app).
  // As regras e segredos de verdade ficam no Rust e no servidor; isto é uma camada a mais.
  build: {
    minify: "terser",
    sourcemap: false,
    terserOptions: { compress: { drop_console: true, drop_debugger: true, passes: 2 }, mangle: { toplevel: true }, format: { comments: false } },
    rollupOptions: {
      plugins: command === "build" ? [obfuscator({
        global: false,
        options: {
          compact: true,
          controlFlowFlattening: true,
          controlFlowFlatteningThreshold: 0.5,
          stringArray: true,
          stringArrayEncoding: ["base64"],
          stringArrayThreshold: 0.75,
          splitStrings: true,
          splitStringsChunkLength: 8,
          identifierNamesGenerator: "hexadecimal",
          selfDefending: false, // incompatível com minificar depois de ofuscar (pode travar)
          disableConsoleOutput: true,
          renameGlobals: false,
          numbersToExpressions: true,
        },
      })] : [],
    },
  },
}));
