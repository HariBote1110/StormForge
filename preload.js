const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('electronAPI', {
  getPlatform: () => process.platform,
  getAppVersion: () => ipcRenderer.invoke('get-app-version'),
  getInitialData: () => ipcRenderer.invoke('get-initial-data'),
  updateSettings: (newSettings) => ipcRenderer.invoke('update-settings', newSettings), // ★ 変更: switchLanguage の代わりに汎用設定更新を使用
  setGameDirectory: () => ipcRenderer.invoke('set-game-directory'),
  autoDetectPath: () => ipcRenderer.invoke('auto-detect-path'),
  launchGame: () => ipcRenderer.invoke('launch-game'),
  addMod: () => ipcRenderer.invoke('add-mod'),
  addModFromPath: (filePath) => ipcRenderer.invoke('add-mod-from-path', filePath),
  // File.path is unavailable in sandboxed renderers; webUtils is the supported API
  getPathForFile: (file) => webUtils.getPathForFile(file),
  deleteMod: (modName) => ipcRenderer.invoke('delete-mod', modName),
  backupRom: () => ipcRenderer.invoke('backup-rom'),
  savePlaylist: (name, activeStates) => ipcRenderer.invoke('save-playlist', name, activeStates),
  loadPlaylist: (name) => ipcRenderer.invoke('load-playlist', name),
  renamePlaylist: (oldName, newName) => ipcRenderer.invoke('rename-playlist', { oldName, newName }),
  deletePlaylist: (name) => ipcRenderer.invoke('delete-playlist', name),
  overwritePlaylist: (name, activeStates) => ipcRenderer.invoke('overwrite-playlist', name, activeStates),
  setLastSelectedPlaylist: (name) => ipcRenderer.invoke('set-last-selected-playlist', name),
  applyModChanges: (activeStates) => ipcRenderer.invoke('apply-mod-changes', activeStates),
  generateShareString: (activeMods) => ipcRenderer.invoke('generate-share-string', activeMods),
  importShareString: (shareString) => ipcRenderer.invoke('import-share-string', shareString),
  copyToClipboard: (text) => ipcRenderer.invoke('copy-to-clipboard', text),
  onShowLoading: (callback) => ipcRenderer.on('show-loading', (event, message) => callback(message)),
  onHideLoading: (callback) => ipcRenderer.on('hide-loading', () => callback())
});