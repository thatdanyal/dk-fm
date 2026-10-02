const { contextBridge, ipcRenderer, webUtils } = require('electron');

const inv = (ch) => (...a) => ipcRenderer.invoke(ch, ...a);

contextBridge.exposeInMainWorld('dk', {
  settings: { get: inv('settings:get'), set: inv('settings:set') },
  library: {
    get: inv('library:get'),
    scan: inv('library:scan'),
    addFiles: inv('library:addFiles'),
    removeTrack: inv('library:removeTrack'),
    like: inv('library:like'),
    played: inv('library:played'),
    upsertPlaylist: inv('library:upsertPlaylist'),
    deletePlaylist: inv('library:deletePlaylist'),
    reveal: inv('library:reveal'),
  },
  dialog: { folder: inv('dialog:folder'), files: inv('dialog:files') },
  shell: { openPath: inv('shell:openPath'), openExternal: inv('shell:openExternal') },
  spotify: { fetch: inv('spotify:fetch') },
  dl: { start: inv('dl:start'), retry: inv('dl:retry'), cancel: inv('dl:cancel'), clear: inv('dl:clear'), list: inv('dl:list') },
  yt: { status: inv('yt:status'), ensure: inv('yt:ensure') },
  lyrics: inv('lyrics:get'),
  win: { minimize: inv('win:minimize'), maximize: inv('win:maximize'), close: inv('win:close'), mini: inv('win:mini'), onTop: inv('win:onTop') },
  updater: { check: inv('updater:check'), install: inv('updater:install'), status: inv('updater:status') },
  pathForFile: (f) => webUtils.getPathForFile(f),
  on: (ch, fn) => {
    const l = (_e, p) => fn(p);
    ipcRenderer.on(ch, l);
    return () => ipcRenderer.removeListener(ch, l);
  },
});
