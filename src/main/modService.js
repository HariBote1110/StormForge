const { app, dialog } = require('electron');
const path = require('path');
const fs = require('fs-extra');
const AdmZip = require('adm-zip');
const { parseStringPromise } = require('xml2js');
const { readStore, writeStore } = require('./store');

const vanillaRomBackupPath = path.join(app.getPath('userData'), 'vanilla_rom_backup');

const MOD_FOLDERS = ['Meshes', 'Definitions', 'Audio', 'Graphics', 'Data'];

function getRomPath(gameDirectory) {
    if (!gameDirectory) return null;

    if (path.basename(gameDirectory) === 'rom') {
        console.warn(`[Mod Service] getRomPath was called with a path that already ends in 'rom': ${gameDirectory}`);
        return gameDirectory;
    }

    if (process.platform === 'darwin') { // macOS
        return path.join(gameDirectory, 'Contents', 'Resources', 'rom');
    }
    // Windows and other platforms
    return path.join(gameDirectory, 'rom');
}

async function installMod(mod, romPath) {
    console.time(`[Performance] Install Mod: ${mod.name}`);
    const installedFiles = [];
    
    // ★ 修正: 並列処理(Promise.all)をやめ、順次処理(for...of)に戻してディスクI/Oの競合を防ぐ
    for (const folder of MOD_FOLDERS) {
        const sourceDir = path.join(mod.path, folder);
        if (await fs.pathExists(sourceDir)) {
            const destDir = path.join(romPath, folder.toLowerCase());
            await fs.ensureDir(destDir);
            
            try {
                await fs.copy(sourceDir, destDir, { overwrite: true });
                
                const files = await fs.readdir(sourceDir);
                const filePaths = files.map(file => path.join(destDir, file));
                installedFiles.push(...filePaths);
            } catch (error) {
                console.error(`[Mod Service] Error copying folder '${folder}' for mod '${mod.name}':`, error);
                throw error;
            }
        }
    }

    console.timeEnd(`[Performance] Install Mod: ${mod.name}`);
    return installedFiles;
}

async function rebuildRomFromActiveMods(mainWindow, translations) {
    const store = readStore();
    console.log(`[Mod Service] Reading gameDirectory from store: ${store.gameDirectory}`);
    const romPath = getRomPath(store.gameDirectory);

    if (!romPath) {
        const errorMsg = "Game directory is not set.";
        console.error(`[Mod Service] ${errorMsg}`);
        throw new Error(errorMsg);
    }
    if (!(await fs.pathExists(vanillaRomBackupPath))) {
        const errorMsg = 'Vanilla ROM backup not found.';
        console.error(`[Mod Service] ${errorMsg}`);
        throw new Error(errorMsg);
    }
    
    console.log('[Mod Service] Starting ROM rebuild process...');
    mainWindow.webContents.send('show-loading', translations.RESTORING_AND_REAPPLYING);

    console.time('[Performance] Total Rebuild Time');

    const useFastCopy = store.settings?.fastCopy !== false;

    try {
        const activeMods = (store.mods || []).filter(m => m.active);

        if (useFastCopy) {
            console.log(`[Mod Service] Calculating folders to restore (Smart Fast Copy)...`);
            
            console.time('[Performance] Smart Fast Copy: Calculation');
            
            const foldersToRestore = new Set();

            // 1. 過去の履歴から特定
            if (store.installedFiles) {
                const allInstalledFiles = Object.values(store.installedFiles).flat();
                for (const filePath of allInstalledFiles) {
                    const relativePath = path.relative(romPath, filePath);
                    if (!relativePath.startsWith('..') && !path.isAbsolute(relativePath)) {
                        const topLevelFolder = relativePath.split(path.sep)[0];
                        const matchedFolder = MOD_FOLDERS.find(f => f.toLowerCase() === topLevelFolder.toLowerCase());
                        if (matchedFolder) {
                            foldersToRestore.add(matchedFolder);
                        }
                    }
                }
            }

            // 2. 今回のMODから特定
            // ここは計算だけなので並列でもOK
            const modCheckPromises = activeMods.map(async (mod) => {
                for (const folder of MOD_FOLDERS) {
                    const sourceDir = path.join(mod.path, folder);
                    if (await fs.pathExists(sourceDir)) {
                        foldersToRestore.add(folder);
                    }
                }
            });
            await Promise.all(modCheckPromises);
            
            console.timeEnd('[Performance] Smart Fast Copy: Calculation');
            console.log(`[Mod Service] Folders to restore:`, [...foldersToRestore]);

            console.time('[Performance] Smart Fast Copy: Restore');
            
            // ★ 修正: 復元処理も順次処理に変更
            for (const folder of foldersToRestore) {
                const folderName = folder.toLowerCase();
                const romTargetDir = path.join(romPath, folderName);
                
                let backupSourceDir = path.join(vanillaRomBackupPath, folderName);
                if (!(await fs.pathExists(backupSourceDir))) {
                    const altBackupSourceDir = path.join(vanillaRomBackupPath, folder);
                    if (await fs.pathExists(altBackupSourceDir)) {
                        backupSourceDir = altBackupSourceDir;
                    } else {
                        continue; 
                    }
                }

                if (await fs.pathExists(romTargetDir)) {
                    await fs.remove(romTargetDir);
                }
                await fs.copy(backupSourceDir, romTargetDir);
            }

            console.timeEnd('[Performance] Smart Fast Copy: Restore');

        } else {
            console.log(`[Mod Service] Performing full ROM restore (Fast Copy Disabled)...`);
            
            console.time('[Performance] Full Restore');
            console.log(`[Mod Service] Clearing ROM directory: ${romPath}`);
            await fs.emptyDir(romPath);
            console.log(`[Mod Service] Restoring vanilla ROM from backup: ${vanillaRomBackupPath}`);
            await fs.copy(vanillaRomBackupPath, romPath);
            console.timeEnd('[Performance] Full Restore');
        }

        store.installedFiles = {};
        console.log(`[Mod Service] Found ${activeMods.length} active mods to install.`);

        console.time('[Performance] All Mods Installation');
        
        for (const activeMod of activeMods) {
            const installedFiles = await installMod(activeMod, romPath);
            store.installedFiles[activeMod.name] = installedFiles;
        }
        
        console.timeEnd('[Performance] All Mods Installation');
        
        writeStore(store);
        console.log('[Mod Service] Successfully rebuilt ROM from active mods.');
    } catch (error) {
        console.error('[Mod Service] An error occurred during ROM rebuild:', error);
        throw error;
    } finally {
        console.timeEnd('[Performance] Total Rebuild Time');
    }
}

async function backupRom(mainWindow, translations) {
    const store = readStore();
    const romPath = getRomPath(store.gameDirectory);

    if (!romPath || !(await fs.pathExists(romPath))) {
        dialog.showErrorBox(translations.ERROR, translations.NOT_SET);
        return { success: false, message: 'Game directory not set.' };
    }

    try {
        console.log('[Mod Service] Starting vanilla ROM backup...');
        mainWindow.webContents.send('show-loading', translations.BACKING_UP);
        
        console.time('[Performance] Backup ROM');
        await fs.emptyDir(vanillaRomBackupPath);
        await fs.copy(romPath, vanillaRomBackupPath);
        console.timeEnd('[Performance] Backup ROM');
        
        console.log('[Mod Service] ROM backup successful.');
        dialog.showMessageBox({ type: 'info', title: translations.SUCCESS, message: translations.ROM_BACKUP_SUCCESS });
        return { success: true };
    } catch (error) {
        console.error('[Mod Service] Failed to backup ROM:', error);
        dialog.showErrorBox(translations.ERROR, error.message);
        return { success: false, message: error.message };
    } finally {
        if(mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send('hide-loading');
    }
}

/**
 * Extracts a .slp/.zip mod archive from the given file path, parses its
 * metadata, and registers it in the store. This is the core logic shared
 * between the dialog-driven "add-mod" IPC handler and future drag & drop
 * support.
 *
 * @param {string} filePath Absolute path to the .slp/.zip archive to install.
 * @returns {Promise<{success: boolean, mod?: object, message?: string}>}
 */
async function addModFromPath(filePath) {
    const modName = path.basename(filePath, path.extname(filePath));
    const modsDir = path.join(app.getPath('userData'), 'mods');
    const extractPath = path.join(modsDir, modName);
    try {
        const tempExtractPath = path.join(modsDir, `__temp_${modName}`);
        await fs.ensureDir(tempExtractPath);
        const zip = new AdmZip(filePath);
        // ★ 修正: 同期版のextractAllToはメインプロセスをブロックしIPC全体を止めてしまうため、非同期版を使用
        await new Promise((resolve, reject) => {
            zip.extractAllToAsync(tempExtractPath, true, false, (err) => err ? reject(err) : resolve());
        });
        const files = await fs.readdir(tempExtractPath);
        let modRootPath = tempExtractPath;
        if (files.length === 1 && (await fs.stat(path.join(tempExtractPath, files[0]))).isDirectory()) { modRootPath = path.join(tempExtractPath, files[0]); }
        await fs.ensureDir(extractPath);
        await fs.copy(modRootPath, extractPath);
        await fs.remove(tempExtractPath);
        const metadataPath = path.join(extractPath, 'Metadata.xml');
        let author = 'Unknown', version = 'Unknown';
        if (await fs.pathExists(metadataPath)) {
            const xmlData = await fs.readFile(metadataPath, 'utf8');
            const parsedData = await parseStringPromise(xmlData);
            author = parsedData.Metadata.Author[0];
            version = parsedData.Metadata.Version[0];
        }
        const modInfo = { name: modName, path: extractPath, author: author, version: version, active: false };
        const store = readStore();
        if (!store.mods) store.mods = [];
        const existingIndex = store.mods.findIndex(m => m.name === modName);
        if (existingIndex > -1) { store.mods[existingIndex] = modInfo; } else { store.mods.push(modInfo); }
        writeStore(store);
        return { success: true, mod: modInfo };
    } catch (error) {
        console.error(`Failed to process mod: ${error}`);
        return { success: false, message: `Failed to process: ${error.message}` };
    }
}

module.exports = { installMod, rebuildRomFromActiveMods, backupRom, getRomPath, addModFromPath };