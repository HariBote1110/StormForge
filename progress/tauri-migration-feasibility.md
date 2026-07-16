# 脱Electron（Tauri移行）の実現可能性調査

## 結論
**実現可能性は高い。** 移行先の第一候補は **Tauri v2**。フロントエンドは素のHTML/CSS/JS（フレームワーク・バンドラなし）のためほぼそのまま流用でき、書き換えが必要なのはメインプロセス約700行（IPC・ファイル操作・ZIP展開・XML解析）のRust化のみ。規模的には数日〜1週間程度の作業量。

## 現状の依存とTauriでの対応

| 現在 (Electron/Node) | Tauri v2 での代替 |
| :--- | :--- |
| レンダラ (index.html + src/renderer/*) | ほぼそのまま。`window.electronAPI.*` を `invoke()` (@tauri-apps/api) に置換 |
| fs-extra (コピー/削除/JSON) | Rust std::fs + fs_extra クレート |
| adm-zip (.slp展開) | zip クレート（非同期化も容易） |
| xml2js (Metadata.xml) | quick-xml + serde |
| pako (共有文字列のdeflate) | flate2（zlib互換なので既存の共有文字列とも相互運用可） |
| dialog.showOpenDialog / showMessageBox | tauri-plugin-dialog |
| shell.openExternal (steam://) | tauri-plugin-opener |
| electron-updater | tauri-plugin-updater（署名鍵の生成と更新マニフェスト形式の変更が必要） |
| store.json (userData) | 同等（app_data_dir）。既存storeの読み込み互換を保てばユーザーデータ移行不要 |
| app.relaunch()（言語切替） | tauri-plugin-process の relaunch |

## 得られるメリット
- 配布サイズ: ~200MB (Electron) → 5MB前後
- メモリ使用量・起動時間の大幅削減
- Rust側で重いコピー処理を別スレッド化でき、UIフリーズ問題が構造的に解消

## リスク / コスト
- Rustの学習コスト（最大の障壁）
- electron-updater → tauri-updater の移行（既存ユーザーの自動更新経路が切れるため、最後のElectron版で新配布先へ誘導する等のブリッジが必要）
- macOSはWKWebView描画になるが、本アプリのUIは単純なため影響は軽微
- Windows版はWebView2ランタイム依存（Win11は標準搭載）

## 検討した代替案
- **Neutralino**: 軽量だがエコシステム・updaterが弱く却下
- **Wails (Go)**: 有力だがmacOSでの実績・プラグイン群でTauriに劣る
- **Electron継続 + 最適化**: フリーズ問題は非同期化で緩和済み。急いで移行する必然性はなく、サイドタスクとして段階移行が妥当

## 発展: 脱Web Stack（WebViewごと廃止する場合）

理想形は **Rust一本化（コアロジック＝Rustクレート + Slint または iced）**。Tauri移行で作るRustバックエンドをそのまま共有ライブラリ化し、UI層だけをSlintに差し替えれば「脱Electron → 脱Web」が連続した一本の道になる。Flutterは開発速度で勝るがロジック層がDartに分断されFFIブリッジが必要になるため次点。SwiftUI+WinUI3の各OSネイティブ2本立ては個人開発では維持コスト過大。

```
Electron ──→ ①Tauri v2 (UI流用・ロジックRust化) ──→ ②Slint (UIだけ差し替え)
```

①だけで止まっても配布サイズ・メモリは大幅改善する。②まで行くとWebView依存（Win側のWebView2ランタイム含む）が完全に消え、ランタイム不要の単一バイナリになる。UIとロジックの同時書き直しはリスクが高いため、一気に②へ飛ばずTauriを中間ステップにするのを推奨。

## 決定 (2026-07-16): WebStack版とRust Native版の2本立てリリース

ユーザー決定により、最終的に **Tauri版（WebStack）と Slint等のRustネイティブ版の両方をリリース**する方針。維持可能にするための必須条件はコアロジックの共有:

```
stormforge/ (Cargo workspace)
├── crates/stormforge-core/   # 共有ロジック: store, mod install, ZIP, XML, Steam検出, 共有文字列
├── apps/tauri/               # WebStack版 (既存HTML/JS UIを流用)
└── apps/native/              # Rustネイティブ版 (Slint)
```

UI層は薄く保ち、機能追加は必ずcoreに入れて両フロントから呼ぶ。

## macOS署名について

- 本プロジェクトは `identity: null`（ad-hoc署名）で配布中。公証には Apple Developer Program ($99/年) が必要で、小規模OSSでは未加入が多数派
- macOS 15 (Sequoia) 以降「右クリック→開く」が塞がれ、システム設定の「このまま開く」または `xattr -d com.apple.quarantine` が必要
- 現実解: ①READMEに回避手順明記（現状維持）→ ②Homebrew Cask登録（無料・UX改善、優先度高）→ ③ユーザーが増えたら公証を検討

## 推奨する進め方
1. 現行Electron版の改善（フリーズ解消・D&D・プレイリストUX）を先に完了
2. `tauri init` で別ブランチに骨組みを作り、レンダラを流用して1コマンド（add-mod）だけRust実装するスパイクで感触を確認
3. スパイクが良好ならIPCハンドラを1つずつ移植
