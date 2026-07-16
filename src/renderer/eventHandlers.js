import { elements, refreshModListUI, refreshPlaylistUI, updatePendingState, addModToList, showToast } from './ui.js';

export function setupEventListeners(translations) {
    // ★ 設定ボタンの挙動変更: モーダルを開く
    elements.settingsBtn.addEventListener('click', async () => {
        const { store } = await window.electronAPI.getInitialData();
        const currentLang = store.settings?.language || 'ja';
        const fastCopy = store.settings?.fastCopy !== false; // デフォルトtrue

        elements.settingsLanguageSelect.value = currentLang;
        elements.settingsFastCopyCheck.checked = fastCopy;
        
        elements.settingsModal.style.display = 'flex';
    });

    // ★ 設定モーダル: 閉じるボタン
    elements.closeSettingsModalBtn.addEventListener('click', () => {
        elements.settingsModal.style.display = 'none';
    });

    // ★ 設定モーダル: 保存ボタン
    elements.saveSettingsBtn.addEventListener('click', async () => {
        const newSettings = {
            language: elements.settingsLanguageSelect.value,
            fastCopy: elements.settingsFastCopyCheck.checked
        };

        const result = await window.electronAPI.updateSettings(newSettings);
        
        if (result.success) {
            if (result.restarting) {
                // 再起動中のため何もしない（またはメッセージ表示）
            } else {
                elements.settingsModal.style.display = 'none';
                showToast(translations.SUCCESS || "Settings saved.", 'success');
            }
        } else {
            showToast("Failed to save settings.", 'error');
        }
    });

    elements.addModBtn.addEventListener('click', async () => {
        elements.loadingMessage.textContent = translations.PROCESSING || "Processing...";
        elements.loadingOverlay.style.display = 'flex';
        try {
            const result = await window.electronAPI.addMod();
            if (result.success) {
                const existingLi = document.querySelector(`li[data-mod-name="${result.mod.name}"]`);
                if (existingLi) {
                    elements.modList.removeChild(existingLi);
                }
                addModToList(result.mod, translations);
                updatePendingState(true);
            } else {
                showToast(`${translations.ERROR}: ${result.message}`, 'error');
            }
        } finally {
            elements.loadingOverlay.style.display = 'none';
        }
    });

    elements.setGameDirBtn.addEventListener('click', async () => {
        const result = await window.electronAPI.setGameDirectory();
        if (result.success) {
            elements.gameDirDisplay.textContent = result.path;
            elements.gameDirContainer.classList.remove('path-hidden'); // 設定後は一度表示する
        }
    });

    elements.autoDetectBtn.addEventListener('click', async () => {
        window.electronAPI.onShowLoading(() => {}); 
        elements.loadingOverlay.style.display = 'flex';
        elements.loadingMessage.textContent = translations.PROCESSING || "Processing...";

        const result = await window.electronAPI.autoDetectPath();
        
        elements.loadingOverlay.style.display = 'none';

        if (result.success) {
            elements.gameDirDisplay.textContent = result.romPath;
            elements.gameDirContainer.classList.remove('path-hidden');
            showToast(translations.SUCCESS || "Path detected successfully!", 'success');
        } else {
            showToast(`${translations.ERROR}: ${result.message}`, 'error');
        }
    });

    elements.launchGameBtn.addEventListener('click', async () => {
        const result = await window.electronAPI.launchGame();
        if (!result.success) {
            showToast(`${translations.ERROR}: ${result.message}`, 'error');
        }
    });

    elements.backupRomBtn.addEventListener('click', () => window.electronAPI.backupRom());

    elements.savePlaylistBtn.addEventListener('click', async () => {
        const name = elements.newPlaylistNameInput.value;
        if (!name) return;
        
        const activeStates = {};
        document.querySelectorAll('#mod-list li').forEach(li => {
            const modName = li.dataset.modName;
            const checkbox = li.querySelector('input[type="checkbox"]');
            activeStates[modName] = checkbox.checked;
        });

        const result = await window.electronAPI.savePlaylist(name, activeStates);
        if (result.success) {
            showToast(result.message, 'success');
            elements.newPlaylistNameInput.value = '';
            const { store } = await window.electronAPI.getInitialData();
            refreshPlaylistUI(store.playlists, name);
            window.electronAPI.setLastSelectedPlaylist(name);
        } else {
            showToast(`${translations.ERROR}: ${result.message}`, 'error');
        }
    });

    // プレイリスト一覧の操作はイベントデリゲーションで処理する
    elements.playlistList.addEventListener('click', async (event) => {
        const button = event.target.closest('.playlist-action-btn');
        if (!button) return;
        const li = button.closest('li');
        const name = li?.dataset.playlistName;
        if (!name) return;

        const refreshList = async () => {
            const { store } = await window.electronAPI.getInitialData();
            refreshPlaylistUI(store.playlists, store.selectedPlaylist);
        };

        switch (button.dataset.action) {
            case 'load': {
                const confirmMessage = translations.LOAD_CONFIRM || "You have unsaved changes. Are you sure you want to load a playlist and discard them?";
                if (window.hasPendingChanges && !confirm(confirmMessage)) return;

                const result = await window.electronAPI.loadPlaylist(name);
                if (result.success) {
                    showToast(result.message, 'success');
                    refreshModListUI(result.mods, translations);
                    updatePendingState(false);
                    await window.electronAPI.setLastSelectedPlaylist(name);
                    await refreshList();
                } else {
                    showToast(`${translations.ERROR}: ${result.message}`, 'error');
                }
                break;
            }
            case 'overwrite': {
                const confirmMessage = (translations.OVERWRITE_CONFIRM || "Are you sure you want to overwrite '{playlistName}'?").replace('{playlistName}', name);
                if (!confirm(confirmMessage)) return;

                const activeStates = {};
                document.querySelectorAll('#mod-list li').forEach(modLi => {
                    const modName = modLi.dataset.modName;
                    const checkbox = modLi.querySelector('input[type="checkbox"]');
                    activeStates[modName] = checkbox.checked;
                });

                const result = await window.electronAPI.overwritePlaylist(name, activeStates);
                if (result.success) {
                    showToast(result.message, 'success');
                    await refreshList();
                } else {
                    showToast(`${translations.ERROR}: ${result.message}`, 'error');
                }
                break;
            }
            case 'rename': {
                const newName = prompt(translations.RENAME_PROMPT || "Enter the new playlist name:", name);
                if (!newName || newName === name) return;

                const result = await window.electronAPI.renamePlaylist(name, newName);
                if (result.success) {
                    showToast(result.message, 'success');
                    await window.electronAPI.setLastSelectedPlaylist(newName);
                    await refreshList();
                } else {
                    showToast(`${translations.ERROR}: ${result.message}`, 'error');
                }
                break;
            }
            case 'delete': {
                const confirmMessage = (translations.DELETE_CONFIRM || "Are you sure you want to delete '{playlistName}'?").replace('{playlistName}', name);
                if (!confirm(confirmMessage)) return;

                const result = await window.electronAPI.deletePlaylist(name);
                if (result.success) {
                    showToast(result.message, 'success');
                    const wasLoaded = li.classList.contains('loaded');
                    if (wasLoaded) {
                        await window.electronAPI.setLastSelectedPlaylist(null);
                    }
                    await refreshList();
                } else {
                    showToast(`${translations.ERROR}: ${result.message}`, 'error');
                }
                break;
            }
        }
    });

    elements.applyChangesBtn.addEventListener('click', async () => {
        const activeStates = {};
        document.querySelectorAll('#mod-list li').forEach(li => {
            const name = li.dataset.modName;
            const checkbox = li.querySelector('input[type="checkbox"]');
            activeStates[name] = checkbox.checked;
        });

        const result = await window.electronAPI.applyModChanges(activeStates);
        if (result.success) {
            updatePendingState(false);
        } else {
            showToast(`${translations.ERROR}: ${result.message}`, 'error');
        }
    });
    
    elements.modList.addEventListener('click', async (event) => {
        if (event.target.closest('.delete-mod-btn')) {
            const li = event.target.closest('li');
            const modName = li.dataset.modName;
            
            const confirmMessage = (translations.DELETE_MOD_CONFIRM || "Are you sure you want to delete '{modName}'? This action cannot be undone.").replace('{modName}', modName);
            if (confirm(confirmMessage)) {
                const result = await window.electronAPI.deleteMod(modName);
                if (result.success) {
                    li.remove();
                    showToast(result.message, 'success');
                } else {
                    showToast(`${translations.ERROR}: ${result.message}`, 'error');
                }
            }
        }
    });

    elements.gameDirContainer.addEventListener('click', () => {
        if (elements.gameDirDisplay.textContent !== (translations.NOT_SET || 'Not set')) {
            const container = elements.gameDirContainer;
            container.classList.toggle('path-hidden');
            const eyeIcon = container.querySelector('.eye-icon img');
            if (container.classList.contains('path-hidden')) {
                eyeIcon.src = 'assets/icons/eye.svg';
            } else {
                eyeIcon.src = 'assets/icons/eye-off.svg';
            }
        }
    });

    elements.shareConfigBtn.addEventListener('click', async () => {
        const activeMods = [];
        const { store } = await window.electronAPI.getInitialData();
        const allMods = store.mods || [];

        document.querySelectorAll('#mod-list li').forEach(li => {
            const checkbox = li.querySelector('input[type="checkbox"]');
            if (checkbox.checked) {
                const modName = li.dataset.modName;
                const mod = allMods.find(m => m.name === modName);
                if (mod) {
                    activeMods.push({ name: mod.name, version: mod.version });
                }
            }
        });

        if (activeMods.length === 0) {
            showToast('No active mods to share.', 'info');
            return;
        }

        const result = await window.electronAPI.generateShareString(activeMods);
        if (result.success) {
            elements.exportTextarea.value = result.shareString;
            elements.exportModal.style.display = 'flex';
        }
    });

    elements.importFromTextBtn.addEventListener('click', () => {
        elements.importTextarea.value = '';
        elements.importModal.style.display = 'flex';
    });

    elements.closeExportModalBtn.addEventListener('click', () => {
        elements.exportModal.style.display = 'none';
    });
    elements.closeImportModalBtn.addEventListener('click', () => {
        elements.importModal.style.display = 'none';
    });

    elements.copyConfigBtn.addEventListener('click', () => {
        window.electronAPI.copyToClipboard(elements.exportTextarea.value);
        showToast('Copied to clipboard!', 'success');
    });

    elements.importConfigBtn.addEventListener('click', async () => {
        const shareString = elements.importTextarea.value;
        if (!shareString) return;

        const result = await window.electronAPI.importShareString(shareString);
        
        if (result.success) {
            const playlistName = `Imported-${new Date().toISOString().slice(0,10)}`;
            const activeStates = {};
            const requiredMods = result.config.mods.map(m => m.name);
            
            document.querySelectorAll('#mod-list li').forEach(li => {
                const modName = li.dataset.modName;
                activeStates[modName] = requiredMods.includes(modName);
            });
            
            const saveResult = await window.electronAPI.savePlaylist(playlistName, activeStates);
            if (saveResult.success) {
                const { store } = await window.electronAPI.getInitialData();
                refreshPlaylistUI(store.playlists, playlistName);
                
                document.querySelectorAll('#mod-list li').forEach(li => {
                    const modName = li.dataset.modName;
                    const checkbox = li.querySelector('input[type="checkbox"]');
                    checkbox.checked = activeStates[modName] || false;
                });

                updatePendingState(true);
                showToast((translations.IMPORT_SUCCESS_DESC || "Playlist '{playlistName}' has been created...").replace('{playlistName}', playlistName), 'success');
                elements.importModal.style.display = 'none';
            }

        } else if (result.missing) {
            let missingModsMessage = `${translations.MISSING_MODS_DESC || 'The following mods are required:'}\n\n`;
            missingModsMessage += result.mods.join('\n');
            showToast(`${translations.MISSING_MODS_TITLE || 'Missing Mods'}: ${missingModsMessage}`, 'error');
        } else {
            showToast(`${translations.ERROR}: ${result.message}`, 'error');
        }
    });

    // ドラッグ&ドロップによるMOD追加
    const dropOverlay = document.getElementById('drop-overlay');
    const dropOverlayMessage = document.getElementById('drop-overlay-message');
    dropOverlayMessage.textContent = translations.DROP_TO_ADD || 'Drop here to add mods';

    // dragleaveは子要素間の移動でも発火するためカウンタで管理する
    let dragCounter = 0;

    const hideDropOverlay = () => {
        dragCounter = 0;
        dropOverlay.classList.remove('drop-overlay-active');
    };

    window.addEventListener('dragenter', (event) => {
        event.preventDefault();
        dragCounter++;
        dropOverlay.classList.add('drop-overlay-active');
    });

    window.addEventListener('dragover', (event) => {
        // デフォルト動作(ファイルへのナビゲーション)を抑止
        event.preventDefault();
    });

    window.addEventListener('dragleave', (event) => {
        event.preventDefault();
        dragCounter--;
        if (dragCounter <= 0) {
            hideDropOverlay();
        }
    });

    window.addEventListener('drop', async (event) => {
        event.preventDefault();
        hideDropOverlay();

        const files = Array.from(event.dataTransfer?.files || []);
        if (files.length === 0) return;

        const modPaths = files
            .map(file => window.electronAPI.getPathForFile(file))
            .filter(filePath => /\.(slp|zip)$/i.test(filePath));

        if (modPaths.length === 0) {
            showToast('Unsupported file type.', 'error');
            return;
        }

        elements.loadingMessage.textContent = translations.PROCESSING || "Processing...";
        elements.loadingOverlay.style.display = 'flex';
        try {
            for (const modPath of modPaths) {
                const result = await window.electronAPI.addModFromPath(modPath);
                if (result.success) {
                    const existingLi = document.querySelector(`li[data-mod-name="${result.mod.name}"]`);
                    if (existingLi) {
                        elements.modList.removeChild(existingLi);
                    }
                    addModToList(result.mod, translations);
                    updatePendingState(true);
                    showToast(result.mod.name, 'success');
                } else {
                    showToast(`${translations.ERROR}: ${result.message}`, 'error');
                }
            }
        } finally {
            elements.loadingOverlay.style.display = 'none';
        }
    });

    window.electronAPI.onShowLoading((message) => {
        elements.loadingMessage.textContent = message;
        elements.loadingOverlay.style.display = 'flex';
    });
    window.electronAPI.onHideLoading(() => {
        elements.loadingOverlay.style.display = 'none';
    });
}