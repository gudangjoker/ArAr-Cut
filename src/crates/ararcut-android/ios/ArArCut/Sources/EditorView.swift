// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors

import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

enum EditorSheet: Identifiable {
    case proAudio
    case cameraLog
    case captions
    case exportVideo
    case nleExport
    case clipAdjust

    var id: Int { hashValue }
}

struct EditorView: View {
    @ObservedObject var store: EditorStore
    let onClose: () -> Void

    @State private var activeSheet: EditorSheet?
    @State private var pickerItems: [PhotosPickerItem] = []
    @State private var importAsOverlay = false
    @State private var showAudioImporter = false
    @State private var showPhotosPicker = false

    var body: some View {
        GeometryReader { geo in
            let isLandscapeStudio = geo.size.width > geo.size.height && geo.size.width >= 600
            VStack(spacing: 0) {
                topBar
                if isLandscapeStudio {
                    HStack(spacing: 0) {
                        VStack(spacing: 0) {
                            previewPane
                            transportBar
                        }
                        .frame(width: geo.size.width * 0.48)
                        Divider().background(Color.white.opacity(0.08))
                        VStack(spacing: 0) {
                            timelinePane
                            if store.selectedClip != nil {
                                clipQuickBar
                            }
                            bottomDock
                        }
                        .frame(maxWidth: .infinity)
                    }
                } else {
                    previewPane
                        .frame(height: max(220, geo.size.height * 0.40))
                    transportBar
                    timelinePane
                        .frame(maxHeight: .infinity)
                    if store.selectedClip != nil {
                        clipQuickBar
                    }
                    bottomDock
                }
            }
            .background(ArArPalette.background.ignoresSafeArea())
        }
        .overlay(alignment: .top) {
            if let msg = store.message {
                Text(msg)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 8)
                    .background(Color.black.opacity(0.85))
                    .clipShape(Capsule())
                    .padding(.top, 54)
                    .onAppear {
                        DispatchQueue.main.asyncAfter(deadline: .now() + 2.8) {
                            if store.message == msg { store.message = nil }
                        }
                    }
            }
        }
        .photosPicker(
            isPresented: $showPhotosPicker,
            selection: $pickerItems,
            maxSelectionCount: 15,
            matching: .any(of: [.videos, .images])
        )
        .onChange(of: pickerItems) { items in
            guard !items.isEmpty else { return }
            let chosen = items
            let overlay = importAsOverlay
            pickerItems = []
            Task {
                var urls: [URL] = []
                for item in chosen {
                    if let media = try? await item.loadTransferable(type: TransferableMedia.self) {
                        urls.append(media.url)
                    }
                }
                store.importFiles(urls, asOverlay: overlay, asAudio: false)
            }
        }
        .fileImporter(
            isPresented: $showAudioImporter,
            allowedContentTypes: [.audio, .mpeg4Audio, .mp3, .wav],
            allowsMultipleSelection: true
        ) { result in
            if case let .success(urls) = result {
                store.importFiles(urls, asOverlay: false, asAudio: true)
            }
        }
        .sheet(item: $activeSheet) { sheet in
            switch sheet {
            case .proAudio:
                ProAudioSheet(store: store)
            case .cameraLog:
                CameraLogAndFiltersSheet(store: store)
            case .captions:
                CaptionsAndTextSheet(store: store)
            case .exportVideo:
                ExportVideoSheet(store: store)
            case .nleExport:
                NleExportSheet(store: store)
            case .clipAdjust:
                ClipAdjustSheet(store: store)
            }
        }
    }

    // MARK: - Top Bar

    private var topBar: some View {
        HStack(spacing: 12) {
            Button(action: onClose) {
                Image(systemName: "chevron.left")
                    .font(.system(size: 17, weight: .semibold))
                    .foregroundStyle(ArArPalette.textPrimary)
                    .frame(width: 34, height: 34)
                    .background(ArArPalette.surface)
                    .clipShape(Circle())
            }

            VStack(alignment: .leading, spacing: 1) {
                Text(store.timeline.name)
                    .font(.subheadline.weight(.semibold))
                    .lineLimit(1)
                Text("\(store.timeline.width)×\(store.timeline.height)")
                    .font(.caption2)
                    .foregroundStyle(ArArPalette.textSecondary)
            }

            Spacer()

            Button { store.undo() } label: {
                Image(systemName: "arrow.uturn.backward")
                    .foregroundStyle(store.timeline.canUndo ? ArArPalette.textPrimary : ArArPalette.textSecondary.opacity(0.35))
            }
            .disabled(!store.timeline.canUndo)

            Button { store.redo() } label: {
                Image(systemName: "arrow.uturn.forward")
                    .foregroundStyle(store.timeline.canRedo ? ArArPalette.textPrimary : ArArPalette.textSecondary.opacity(0.35))
            }
            .disabled(!store.timeline.canRedo)

            Button {
                activeSheet = .nleExport
            } label: {
                Image(systemName: "square.and.arrow.up.on.square")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(ArArPalette.textPrimary)
                    .padding(7)
                    .background(ArArPalette.surface)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }

            Button {
                store.stopPlayback()
                activeSheet = .exportVideo
            } label: {
                Text("Export")
                    .font(.caption.weight(.bold))
                    .foregroundStyle(.black)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .background(ArArPalette.accent)
                    .clipShape(Capsule())
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(ArArPalette.background)
    }

    // MARK: - Preview + Live Video Scopes HUD

    private var previewPane: some View {
        ZStack(alignment: .topTrailing) {
            Color.black
            if let img = store.previewImage {
                Image(uiImage: img)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                VStack(spacing: 6) {
                    Image(systemName: "film")
                        .font(.title2)
                        .foregroundStyle(ArArPalette.textSecondary)
                    Text("Add media to preview")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                }
            }

            if let kind = store.scopeKind {
                videoScopesHud(kind: kind)
                    .padding(8)
            }

            if store.isWorking {
                ProgressView()
                    .tint(ArArPalette.accent)
                    .padding(10)
                    .background(Color.black.opacity(0.6))
                    .clipShape(Circle())
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomTrailing)
                    .padding(10)
            }
        }
    }

    private func videoScopesHud(kind: VideoScopeKind) -> some View {
        VStack(spacing: 5) {
            HStack(spacing: 4) {
                ForEach(
                    [
                        (VideoScopeKind.waveformRgb, "WF"),
                        (VideoScopeKind.waveformLuma, "Y"),
                        (VideoScopeKind.paradeRgb, "RGB"),
                        (VideoScopeKind.vectorscopeYuv, "VEC"),
                        (VideoScopeKind.histogram, "HIST"),
                    ],
                    id: \.1
                ) { mode, label in
                    Button {
                        store.scopeKind = mode
                        store.refreshScope()
                    } label: {
                        Text(label)
                            .font(.system(size: 9, weight: .bold))
                            .foregroundStyle(mode == kind ? .black : .white)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 3)
                            .background(mode == kind ? ArArPalette.accent : Color.white.opacity(0.16))
                            .clipShape(RoundedRectangle(cornerRadius: 4))
                    }
                }
                Button {
                    store.scopeKind = nil
                    store.scopeImage = nil
                } label: {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(.white)
                        .padding(4)
                        .background(Color.white.opacity(0.16))
                        .clipShape(RoundedRectangle(cornerRadius: 4))
                }
            }
            ZStack {
                Color.black
                if let img = store.scopeImage {
                    Image(uiImage: img)
                        .resizable()
                        .scaledToFit()
                        .padding(2)
                } else {
                    Text("Sampling…")
                        .font(.caption2)
                        .foregroundStyle(ArArPalette.textSecondary)
                }
            }
            .frame(width: 178, height: kind == .vectorscopeYuv ? 116 : 86)
            .clipShape(RoundedRectangle(cornerRadius: 6))
        }
        .padding(6)
        .background(Color.black.opacity(0.84))
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .overlay(
            RoundedRectangle(cornerRadius: 10)
                .stroke(Color.white.opacity(0.18), lineWidth: 1)
        )
    }

    // MARK: - Transport Bar

    private var transportBar: some View {
        HStack(spacing: 14) {
            Text("\(formatTimecode(store.playheadUs)) / \(formatTimecode(store.timeline.durationUs))")
                .font(.caption.monospacedDigit())
                .foregroundStyle(ArArPalette.textSecondary)

            Spacer()

            Button {
                store.scrub(to: 0, scrubbing: false)
            } label: {
                Image(systemName: "backward.end.fill")
                    .font(.system(size: 14))
                    .foregroundStyle(ArArPalette.textPrimary)
            }

            Button {
                store.togglePlay()
            } label: {
                Image(systemName: store.isPlayingNow ? "pause.fill" : "play.fill")
                    .font(.system(size: 20))
                    .foregroundStyle(.black)
                    .frame(width: 38, height: 38)
                    .background(ArArPalette.accent)
                    .clipShape(Circle())
            }

            Button {
                if store.scopeKind == nil {
                    store.scopeKind = .waveformRgb
                    store.refreshScope()
                } else {
                    store.scopeKind = nil
                    store.scopeImage = nil
                }
            } label: {
                Image(systemName: "waveform.path.ecg")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(store.scopeKind != nil ? .black : ArArPalette.textPrimary)
                    .padding(7)
                    .background(store.scopeKind != nil ? ArArPalette.accent : ArArPalette.surface)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }

            Spacer()

            HStack(spacing: 6) {
                Button {
                    store.pixelsPerSecond = max(24, store.pixelsPerSecond * 0.75)
                } label: {
                    Image(systemName: "minus.magnifyingglass")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                }
                Button {
                    store.pixelsPerSecond = min(220, store.pixelsPerSecond * 1.33)
                } label: {
                    Image(systemName: "plus.magnifyingglass")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(ArArPalette.surface)
    }

    // MARK: - Multi-Track Timeline

    private var timelinePane: some View {
        VStack(spacing: 6) {
            if store.timeline.durationUs > 0 {
                Slider(
                    value: Binding(
                        get: { Double(store.playheadUs) },
                        set: { store.scrub(to: Int64($0), scrubbing: true) }
                    ),
                    in: 0...Double(max(1, store.timeline.durationUs)),
                    onEditingChanged: { editing in
                        if !editing {
                            store.scrub(to: store.playheadUs, scrubbing: false)
                        }
                    }
                )
                .tint(ArArPalette.accent)
                .padding(.horizontal, 14)
            }

            ScrollView([.horizontal, .vertical]) {
                let durSec = max(5.0, Double(store.timeline.durationUs) / 1_000_000.0 + 2.0)
                let totalWidth = max(340, CGFloat(durSec) * store.pixelsPerSecond)
                ZStack(alignment: .topLeading) {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(store.timeline.rows, id: \.trackId) { row in
                            timelineRow(row, totalWidth: totalWidth)
                        }
                    }
                    .padding(.vertical, 8)
                    .padding(.horizontal, 12)

                    let playheadX = 12 + CGFloat(Double(store.playheadUs) / 1_000_000.0) * store.pixelsPerSecond
                    Rectangle()
                        .fill(ArArPalette.accent)
                        .frame(width: 2)
                        .frame(maxHeight: .infinity)
                        .offset(x: playheadX)
                        .allowsHitTesting(false)
                }
                .frame(minWidth: totalWidth + 24, alignment: .topLeading)
            }
            .background(ArArPalette.background)
        }
    }

    private func timelineRow(_ row: RowView, totalWidth: CGFloat) -> some View {
        ZStack(alignment: .leading) {
            RoundedRectangle(cornerRadius: 8)
                .fill(ArArPalette.surface.opacity(0.65))
                .frame(width: totalWidth, height: row.kind == .main ? 52 : 38)

            ForEach(row.clips, id: \.id) { clip in
                let x = CGFloat(Double(clip.startUs) / 1_000_000.0) * store.pixelsPerSecond
                let w = max(36, CGFloat(Double(clip.durationUs) / 1_000_000.0) * store.pixelsPerSecond)
                let selected = store.selectedClipId == clip.id
                Button {
                    store.selectedClipId = clip.id
                } label: {
                    HStack(spacing: 4) {
                        Text(clip.text.isEmpty ? (clip.name.isEmpty ? clip.kind : clip.name) : clip.text)
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 8)
                    .frame(width: w, height: row.kind == .main ? 46 : 32)
                    .background(rowColor(row.kind).opacity(selected ? 0.95 : 0.72))
                    .clipShape(RoundedRectangle(cornerRadius: 6))
                    .overlay(
                        RoundedRectangle(cornerRadius: 6)
                            .stroke(selected ? Color.white : Color.clear, lineWidth: 2)
                    )
                }
                .offset(x: x)
            }
        }
    }

    private func rowColor(_ kind: RowKind) -> Color {
        switch kind {
        case .main: Color(red: 0.16, green: 0.42, blue: 0.68)
        case .overlay: Color(red: 0.35, green: 0.32, blue: 0.75)
        case .audio: Color(red: 0.12, green: 0.55, blue: 0.45)
        case .text: Color(red: 0.75, green: 0.42, blue: 0.16)
        case .effect: Color(red: 0.60, green: 0.24, blue: 0.68)
        case .filter: Color(red: 0.56, green: 0.38, blue: 0.18)
        }
    }

    // MARK: - Selected Clip Quick Actions

    private var clipQuickBar: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                if let clip = store.selectedClip {
                    quickChip("Split", icon: "scissors") {
                        let id = clip.id
                        let at = store.playheadUs
                        store.runEdit { try $0.split(clipIds: [id], atUs: at) }
                    }
                    quickChip("Adjust", icon: "slider.horizontal.3") {
                        activeSheet = .clipAdjust
                    }
                    quickChip("Duplicate", icon: "plus.square.on.square") {
                        let id = clip.id
                        store.runEdit { try $0.duplicate(clipId: id) }
                    }
                    if clip.kind == "video" {
                        quickChip("Reverse", icon: "arrow.uturn.left") {
                            let id = clip.id
                            store.runEdit { try $0.reverse(clipId: id) }
                        }
                        quickChip("Freeze", icon: "snowflake") {
                            let id = clip.id
                            let at = store.playheadUs
                            store.runEdit { try $0.freeze(clipId: id, atUs: at) }
                        }
                        if clip.audible {
                            quickChip("Extract Audio", icon: "waveform.badge.minus") {
                                let id = clip.id
                                store.runEdit { try $0.extractAudio(clipId: id) }
                            }
                        }
                    }
                    quickChip("Delete", icon: "trash", danger: true) {
                        let id = clip.id
                        store.runEdit { try $0.delete(clipIds: [id]) }
                    }
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
        }
        .background(ArArPalette.surfaceElevated)
    }

    private func quickChip(_ title: String, icon: String, danger: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Label(title, systemImage: icon)
                .font(.caption2.weight(.semibold))
                .foregroundStyle(danger ? ArArPalette.danger : ArArPalette.textPrimary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(ArArPalette.surface)
                .clipShape(Capsule())
        }
    }

    // MARK: - Bottom Tool Dock

    private var bottomDock: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 14) {
                dockButton("Media", icon: "plus.rectangle.on.folder") {
                    importAsOverlay = false
                    showPhotosPicker = true
                }
                dockButton("Overlay", icon: "square.on.square") {
                    importAsOverlay = true
                    showPhotosPicker = true
                }
                dockButton("Audio", icon: "music.note") {
                    showAudioImporter = true
                }
                dockButton("Pro Audio", icon: "waveform", accent: true) {
                    activeSheet = .proAudio
                }
                dockButton("Log & LUT", icon: "camera.filters", accent: true) {
                    activeSheet = .cameraLog
                }
                dockButton("Text / Cap", icon: "captions.bubble") {
                    activeSheet = .captions
                }
                dockButton("NLE XML", icon: "square.and.arrow.up") {
                    activeSheet = .nleExport
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
        }
        .background(ArArPalette.surface)
    }

    private func dockButton(_ title: String, icon: String, accent: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 4) {
                Image(systemName: icon)
                    .font(.system(size: 18))
                    .foregroundStyle(accent ? ArArPalette.accent : ArArPalette.textPrimary)
                Text(title)
                    .font(.system(size: 10, weight: .medium))
                    .foregroundStyle(accent ? ArArPalette.accent : ArArPalette.textSecondary)
            }
            .frame(minWidth: 54)
        }
    }
}
