// DOM要素の取得
export const elements = {
    settingsBtn: document.getElementById('settings-btn'),
    addModBtn: document.getElementById('add-mod-btn'),
    setGameDirBtn: document.getElementById('set-game-dir-btn'),
    autoDetectBtn: document.getElementById('auto-detect-btn'),
    launchGameBtn: document.getElementById('launch-game-btn'),
    backupRomBtn: document.getElementById('backup-rom-btn'),
    gameDirContainer: document.getElementById('game-dir-container'),
    gameDirDisplay: document.getElementById('game-dir-display'),
    modList: document.getElementById('mod-list'),
    loadingOverlay: document.getElementById('loading-overlay'),
    loadingMessage: document.getElementById('loading-message'),
    playlistList: document.getElementById('playlist-list'),
    savePlaylistBtn: document.getElementById('save-playlist-btn'),
    newPlaylistNameInput: document.getElementById('new-playlist-name'),
    applyChangesBtn: document.getElementById('apply-changes-btn'),
    pendingIndicator: document.getElementById('pending-indicator'),
    shareConfigBtn: document.getElementById('share-config-btn'),
    importFromTextBtn: document.getElementById('import-from-text-btn'),

    // Settings Modal
    settingsModal: document.getElementById('settings-modal'),
    settingsLanguageSelect: document.getElementById('settings-language-select'),
    settingsFastCopyCheck: document.getElementById('settings-fast-copy-check'),
    saveSettingsBtn: document.getElementById('save-settings-btn'),
    closeSettingsModalBtn: document.getElementById('close-settings-modal-btn'),

    // Other Modals
    exportModal: document.getElementById('export-modal'),
    exportTextarea: document.getElementById('export-textarea'),
    copyConfigBtn: document.getElementById('copy-config-btn'),
    closeExportModalBtn: document.getElementById('close-export-modal-btn'),
    importModal: document.getElementById('import-modal'),
    importTextarea: document.getElementById('import-textarea'),
    importConfigBtn: document.getElementById('import-config-btn'),
    closeImportModalBtn: document.getElementById('close-import-modal-btn'),
};

// ... (残りのコードは変更なし)

export function applyTranslations(translations, platform) {
    document.querySelectorAll('[data-lang]').forEach(el => {
        const key = el.getAttribute('data-lang');
        
        // OS固有のキーがあるかチェック (例: SET_GAME_DIR_WIN)
        const platformKey = `${key}_${platform === 'win32' ? 'WIN' : 'MAC'}`;
        if (translations[platformKey]) {
            el.textContent = translations[platformKey];
        } else if (translations[key]) {
            el.textContent = translations[key];
        }
    });
    document.querySelectorAll('[data-lang-placeholder]').forEach(el => {
        const key = el.getAttribute('data-lang-placeholder');
        if (translations[key]) el.placeholder = translations[key];
    });
    document.querySelectorAll('[data-lang-default]').forEach(el => {
        const currentText = el.textContent.trim();
        if (!currentText || currentText === 'Not set' || currentText === '未設定') {
            const key = el.getAttribute('data-lang-default');
            if (translations[key]) el.textContent = translations[key];
        }
    });
    document.title = translations.APP_TITLE || 'StormForge';
}

export function addModToList(mod, translations) {
    const li = document.createElement('li');
    li.dataset.modName = mod.name;
    const modDetails = `<div><strong>${mod.name}</strong><br><small>${translations.AUTHOR}: ${mod.author} | ${translations.VERSION}: ${mod.version}</small></div>`;
    
    const controls = `
        <div class="mod-controls">
            <label class="switch"><input type="checkbox" ${mod.active ? 'checked' : ''}><span class="slider"></span></label>
            <button class="delete-mod-btn danger"><img src="assets/icons/trash.svg" alt="Delete"></button>
        </div>
    `;

    li.innerHTML = modDetails + controls;

    const checkbox = li.querySelector('input[type="checkbox"]');
    checkbox.addEventListener('change', () => {
        updatePendingState(true);
    });
    
    elements.modList.appendChild(li);
}

export function refreshModListUI(mods, translations) {
    elements.modList.innerHTML = '';
    if (mods) {
        mods.forEach(mod => addModToList(mod, translations));
    }
}

// refreshPlaylistUI is called without translations from event handlers,
// so cache the dictionary supplied to initializeUI at module level
let cachedTranslations = {};

export function refreshPlaylistUI(playlists, selected) {
    const t = cachedTranslations;
    elements.playlistList.innerHTML = '';

    const names = playlists ? Object.keys(playlists) : [];
    if (names.length === 0) {
        const li = document.createElement('li');
        li.className = 'playlist-empty';
        li.textContent = t.NO_PLAYLISTS || 'No playlists yet';
        elements.playlistList.appendChild(li);
        return;
    }

    names.forEach(name => {
        const li = document.createElement('li');
        li.dataset.playlistName = name;
        const isLoaded = name === selected;
        if (isLoaded) li.classList.add('loaded');

        const nameSpan = document.createElement('span');
        nameSpan.className = 'playlist-name';
        nameSpan.textContent = name;
        li.appendChild(nameSpan);

        if (isLoaded) {
            const badge = document.createElement('span');
            badge.className = 'playlist-badge';
            badge.textContent = t.LOADED_BADGE || 'Loaded';
            li.appendChild(badge);
        }

        const actions = document.createElement('div');
        actions.className = 'playlist-actions';
        const buttons = [
            { action: 'load', symbol: '▶', title: t.LOAD_PLAYLIST || 'Load' },
            { action: 'overwrite', symbol: '💾', title: t.OVERWRITE_PLAYLIST || 'Overwrite with current state' },
            { action: 'rename', symbol: '✎', title: t.RENAME_PLAYLIST || 'Rename' },
            { action: 'delete', symbol: '', title: t.DELETE_PLAYLIST || 'Delete', danger: true },
        ];
        buttons.forEach(({ action, symbol, title, danger }) => {
            const btn = document.createElement('button');
            btn.className = `playlist-action-btn${danger ? ' danger' : ''}`;
            btn.dataset.action = action;
            btn.title = title;
            if (action === 'delete') {
                const img = document.createElement('img');
                img.src = 'assets/icons/trash.svg';
                img.alt = title;
                btn.appendChild(img);
            } else {
                btn.textContent = symbol;
            }
            actions.appendChild(btn);
        });
        li.appendChild(actions);

        elements.playlistList.appendChild(li);
    });
}

export function updatePendingState(isPending) {
    window.hasPendingChanges = isPending;
    elements.pendingIndicator.style.display = isPending ? 'inline' : 'none';
    elements.applyChangesBtn.style.display = isPending ? 'block' : 'none';
}

/**
 * Shows a non-blocking toast notification instead of alert(), which freezes
 * the renderer's event loop until dismissed.
 * @param {string} message Text to display.
 * @param {'info'|'success'|'error'} [type='info'] Visual style of the toast.
 */
export function showToast(message, type = 'info') {
    const container = document.getElementById('toast-container');
    if (!container) return;

    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;
    toast.textContent = message;
    container.appendChild(toast);

    // 次のフレームでクラスを付与しトランジションを発火させる
    requestAnimationFrame(() => {
        toast.classList.add('toast-visible');
    });

    const dismiss = () => {
        toast.classList.remove('toast-visible');
        toast.addEventListener('transitionend', () => toast.remove(), { once: true });
    };

    setTimeout(dismiss, 4000);
}

export function initializeUI(translations, store, platform) {
    cachedTranslations = translations || {};
    applyTranslations(translations, platform);
    if (store.romPath) {
        elements.gameDirDisplay.textContent = store.romPath;
        elements.gameDirContainer.classList.add('path-hidden');
    } else {
        elements.gameDirContainer.classList.add('path-hidden');
        elements.gameDirDisplay.textContent = translations.NOT_SET || 'Not set';
    }
    refreshModListUI(store.mods, translations);
    refreshPlaylistUI(store.playlists, store.selectedPlaylist);
    updatePendingState(false);
}