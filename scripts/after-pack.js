// electron-builder afterPack hook: drop Chromium files DK.FM never uses (~32 MB on Windows).
//  - dxcompiler.dll / dxil.dll: DirectX shader compiler for WebGPU (we don't use WebGPU)
//  - vk_swiftshader*, vulkan-1.dll: software Vulkan for WebGL/WebGPU fallback (no WebGL here)
// Verified on Windows: renders, plays, and composites with and without a GPU.
const fs = require('fs');
const path = require('path');

const REMOVE = {
  win32: ['dxcompiler.dll', 'dxil.dll', 'vk_swiftshader.dll', 'vk_swiftshader_icd.json', 'vulkan-1.dll'],
};

exports.default = async function afterPack(ctx) {
  const list = REMOVE[ctx.electronPlatformName] || [];
  let saved = 0;
  for (const f of list) {
    const p = path.join(ctx.appOutDir, f);
    if (fs.existsSync(p)) {
      saved += fs.statSync(p).size;
      fs.rmSync(p);
    }
  }
  if (saved) console.log(`  • afterPack: removed unused Chromium files (${(saved / 1048576).toFixed(1)} MB)`);
};
