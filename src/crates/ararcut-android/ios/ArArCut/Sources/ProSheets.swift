// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors

import Photos
import SwiftUI

// MARK: - 1. Pro Audio DSP Sheet (EBU R128 + Mastering Chains + GCC-PHAT Sync)

struct ProAudioSheet: View {
    @ObservedObject var store: EditorStore
    @Environment(\.dismiss) private var dismiss

    @State private var presets: [ProAudioPreset] = proAudioPresets()
    @State private var targetLufs: Double = -14.0
    @State private var loudness: LoudnessReport?
    @State private var syncReport: AudioSyncReport?
    @State private var hasProcessed = false
    @State private var working: String?

    var body: some View {
        NavigationStack {
            Group {
                if let clip = store.selectedClip {
                    List {
                        Section("EBU R128 / ITU-R BS.1770-4 Loudness Meter") {
                            Button {
                                measureLoudness(path: clip.path)
                            } label: {
                                HStack {
                                    Label(working == "meter" ? "Scanning…" : "Measure Clip Loudness", systemImage: "gauge.medium")
                                    Spacer()
                                    if working == "meter" { ProgressView() }
                                }
                            }
                            .disabled(working != nil)

                            if let m = loudness {
                                HStack {
                                    metricBox("Integrated", String(format: "%.1f LUFS", m.integratedLufs))
                                    metricBox("True Peak", String(format: "%.1f dBTP", m.truePeakDbtp))
                                    metricBox("Range", String(format: "%.1f LU", m.loudnessRangeLu))
                                    metricBox("Short-Term", String(format: "%.1f", m.maxShortTermLufs))
                                }
                            }
                        }

                        Section("Target Loudness") {
                            Picker("Target LUFS", selection: $targetLufs) {
                                Text("-14 Social").tag(-14.0)
                                Text("-16 Voice").tag(-16.0)
                                Text("-23 EBU").tag(-23.0)
                            }
                            .pickerStyle(.segmented)

                            if hasProcessed {
                                Button("Revert to Original Audio", role: .destructive) {
                                    revertAudio(clipId: clip.id)
                                }
                                .disabled(working != nil)
                            }
                        }

                        Section("Mastering & Restoration Chains") {
                            ForEach(presets, id: \.id) { preset in
                                Button {
                                    applyPreset(clipId: clip.id, preset: preset)
                                } label: {
                                    HStack {
                                        VStack(alignment: .leading, spacing: 3) {
                                            Text(preset.name).font(.subheadline.weight(.semibold))
                                            Text(preset.summary).font(.caption).foregroundStyle(ArArPalette.textSecondary)
                                        }
                                        Spacer()
                                        if working == preset.id {
                                            ProgressView()
                                        } else {
                                            Image(systemName: "waveform").foregroundStyle(ArArPalette.accent)
                                        }
                                    }
                                }
                                .disabled(working != nil)
                            }
                        }

                        if let mainRef = store.timeline.rows.first(where: { $0.kind == .main })?.clips.first(where: { $0.audible }),
                           let currentRow = store.selectedRow,
                           currentRow.kind != .main,
                           mainRef.id != clip.id {
                            Section("GCC-PHAT Auto Audio Sync") {
                                Button {
                                    runSync(mainRef: mainRef, clip: clip, trackId: currentRow.trackId)
                                } label: {
                                    HStack {
                                        Label(working == "sync" ? "Syncing…" : "Align to Main Track Audio", systemImage: "arrow.left.and.right.righttriangle.left.righttriangle.right")
                                        Spacer()
                                        if working == "sync" { ProgressView() }
                                    }
                                }
                                .disabled(working != nil)

                                if let s = syncReport {
                                    Text(String(format: "Offset: %+.1f ms · Confidence: %.1f · %@", Double(s.offsetUs) / 1000.0, s.confidence, s.reliable ? "High reliability" : "Check alignment"))
                                        .font(.caption)
                                        .foregroundStyle(s.reliable ? ArArPalette.accent : ArArPalette.textSecondary)
                                }
                            }
                        }
                    }
                    .onAppear {
                        hasProcessed = store.session.hasProAudio(clipId: clip.id)
                    }
                } else {
                    Text("Select a video or audio clip first.")
                        .foregroundStyle(ArArPalette.textSecondary)
                }
            }
            .navigationTitle("Pro Audio DSP")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.large])
    }

    private func metricBox(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption2).foregroundStyle(ArArPalette.textSecondary)
            Text(value).font(.caption.weight(.bold)).foregroundStyle(ArArPalette.accent)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func measureLoudness(path: String) {
        working = "meter"
        Task.detached(priority: .userInitiated) {
            do {
                let report = try analyzeClipLoudness(path: path)
                await MainActor.run {
                    self.working = nil
                    self.loudness = report
                }
            } catch {
                await MainActor.run {
                    self.working = nil
                    self.store.message = error.localizedDescription
                }
            }
        }
    }

    private func applyPreset(clipId: String, preset: ProAudioPreset) {
        working = preset.id
        let sess = store.session
        let lufs = targetLufs
        Task.detached(priority: .userInitiated) {
            do {
                let view = try sess.applyProAudio(clipId: clipId, presetId: preset.id, targetLufs: lufs)
                await MainActor.run {
                    self.working = nil
                    self.hasProcessed = true
                    self.loudness = nil
                    self.store.applyView(view)
                    self.store.message = "Applied \(preset.name)"
                }
            } catch {
                await MainActor.run {
                    self.working = nil
                    self.store.message = error.localizedDescription
                }
            }
        }
    }

    private func revertAudio(clipId: String) {
        working = "revert"
        let sess = store.session
        Task.detached(priority: .userInitiated) {
            do {
                let view = try sess.revertProAudio(clipId: clipId)
                await MainActor.run {
                    self.working = nil
                    self.hasProcessed = false
                    self.loudness = nil
                    self.store.applyView(view)
                    self.store.message = "Reverted to original audio"
                }
            } catch {
                await MainActor.run {
                    self.working = nil
                    self.store.message = error.localizedDescription
                }
            }
        }
    }

    private func runSync(mainRef: ClipView, clip: ClipView, trackId: String) {
        working = "sync"
        Task.detached(priority: .userInitiated) {
            do {
                let report = try measureAudioSync(referencePath: mainRef.path, targetPath: clip.path)
                let aligned = max(0, mainRef.startUs + report.offsetUs)
                await MainActor.run {
                    self.working = nil
                    self.syncReport = report
                    self.store.runEdit { try $0.moveClip(clipId: clip.id, trackId: trackId, startUs: aligned) }
                    self.store.message = String(format: "Synced (%+.1f ms)", Double(report.offsetUs) / 1000.0)
                }
            } catch {
                await MainActor.run {
                    self.working = nil
                    self.store.message = error.localizedDescription
                }
            }
        }
    }
}

// MARK: - 2. Camera Log -> Rec.709 & Built-In Filters Sheet

struct CameraLogAndFiltersSheet: View {
    @ObservedObject var store: EditorStore
    @Environment(\.dismiss) private var dismiss

    @State private var tab = 0
    @State private var logProfiles = cameraLogProfiles()
    @State private var filters = packages(kind: "filter")
    @State private var applying: String?

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                Picker("Mode", selection: $tab) {
                    Text("Camera Log → Rec.709").tag(0)
                    Text("GPU Filters (\(filters.count))").tag(1)
                }
                .pickerStyle(.segmented)
                .padding()

                if tab == 0 {
                    List {
                        Section {
                            Text("Bakes a 33³ 3D LUT with BT.2390 highlight roll-off & ACES gamut compression directly into the Metal/wgpu shader pipeline.")
                                .font(.caption)
                                .foregroundStyle(ArArPalette.textSecondary)
                        }
                        ForEach(logProfiles, id: \.id) { profile in
                            Button {
                                applyCameraLog(profile)
                            } label: {
                                HStack {
                                    Text(profile.camera)
                                        .font(.caption.weight(.bold))
                                        .foregroundStyle(ArArPalette.accent)
                                        .padding(.horizontal, 8)
                                        .padding(.vertical, 4)
                                        .background(ArArPalette.surfaceElevated)
                                        .clipShape(RoundedRectangle(cornerRadius: 6))
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(profile.name).font(.subheadline.weight(.semibold))
                                        Text("\(profile.gamut) → Rec.709").font(.caption2).foregroundStyle(ArArPalette.textSecondary)
                                    }
                                    Spacer()
                                    if applying == profile.id { ProgressView() }
                                }
                            }
                            .disabled(applying != nil)
                        }
                    }
                } else {
                    List(filters, id: \.id) { pkg in
                        Button {
                            applyLookId(pkg.id, displayName: pkg.name)
                        } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(pkg.name).font(.subheadline.weight(.semibold))
                                    Text(pkg.category).font(.caption2).foregroundStyle(ArArPalette.textSecondary)
                                }
                                Spacer()
                                Image(systemName: "sparkles").foregroundStyle(ArArPalette.accent)
                            }
                        }
                    }
                }
            }
            .navigationTitle("Color & Looks")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(store.scopeKind == nil ? "Show Scopes" : "Hide Scopes") {
                        if store.scopeKind == nil {
                            store.scopeKind = .waveformRgb
                            store.refreshScope()
                        } else {
                            store.scopeKind = nil
                            store.scopeImage = nil
                        }
                    }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.large])
    }

    private func applyCameraLog(_ profile: CameraLogProfile) {
        applying = profile.id
        Task.detached(priority: .userInitiated) {
            do {
                let lookId = try installCameraLogLook(profileId: profile.id)
                await MainActor.run {
                    self.applying = nil
                    self.applyLookId(lookId, displayName: "\(profile.camera) \(profile.name)")
                }
            } catch {
                await MainActor.run {
                    self.applying = nil
                    self.store.message = error.localizedDescription
                }
            }
        }
    }

    private func applyLookId(_ lookId: String, displayName: String) {
        if let clip = store.selectedClip, clip.kind == "video" || clip.kind == "image" {
            var existing: [[String: Any]] = []
            if let data = clip.json.data(using: .utf8),
               let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let chain = obj["videoEffects"] as? [[String: Any]] {
                existing = chain.filter { ($0["id"] as? String) != lookId }
            }
            existing.append(["id": lookId, "params": [String: Any]()])
            store.patchClip(clip.id, patch: ["videoEffects": existing])
        } else {
            let at = store.playheadUs
            store.runEdit { try $0.addLayer(effectId: lookId, name: displayName, atUs: at, durationUs: 3_000_000) }
        }
        store.message = "Applied \(displayName)"
    }
}

// MARK: - 3. Text, Captions & Speaker Diarization Sheet

struct CaptionsAndTextSheet: View {
    @ObservedObject var store: EditorStore
    @Environment(\.dismiss) private var dismiss

    @State private var captionText = ""
    @State private var maxSpeakers = 4
    @State private var diarizing = false

    var body: some View {
        NavigationStack {
            Form {
                Section("Add Text / Captions at Playhead") {
                    TextField("Enter title or caption lines…", text: $captionText, axis: .vertical)
                        .lineLimit(2...5)
                    Button("Add Caption Track") {
                        let text = captionText.trimmingCharacters(in: .whitespacesAndNewlines)
                        guard !text.isEmpty else { return }
                        let at = store.playheadUs
                        store.runEdit { try $0.enterCaptions(text: text, atUs: at) }
                        captionText = ""
                        dismiss()
                    }
                    .disabled(captionText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }

                Section("On-Device Speaker Diarization") {
                    Text("Clusters caption clips by voice timbre using classical MFCC clustering (zero model download) and prefixes them with Speaker 1:, Speaker 2:…")
                        .font(.caption)
                        .foregroundStyle(ArArPalette.textSecondary)
                    Picker("Max Speakers", selection: $maxSpeakers) {
                        Text("2").tag(2)
                        Text("3").tag(3)
                        Text("4").tag(4)
                        Text("6").tag(6)
                    }
                    .pickerStyle(.segmented)

                    Button {
                        runDiarize()
                    } label: {
                        HStack {
                            Label(diarizing ? "Clustering Voices…" : "Auto-Label Caption Speakers", systemImage: "person.2.wave.2")
                            Spacer()
                            if diarizing { ProgressView() }
                        }
                    }
                    .disabled(diarizing)
                }
            }
            .navigationTitle("Text & Captions")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func runDiarize() {
        diarizing = true
        let sess = store.session
        let maxSpk = UInt32(maxSpeakers)
        Task.detached(priority: .userInitiated) {
            do {
                let report = try sess.labelCaptionSpeakers(maxSpeakers: maxSpk)
                await MainActor.run {
                    self.diarizing = false
                    self.store.applyView(report.view)
                    self.store.message = "Found \(report.speakersFound) speaker(s) across \(report.captionsLabelled) caption(s)"
                    self.dismiss()
                }
            } catch {
                await MainActor.run {
                    self.diarizing = false
                    self.store.message = error.localizedDescription
                }
            }
        }
    }
}

// MARK: - 4. Clip Volume / Speed / Mute Adjust Sheet

struct ClipAdjustSheet: View {
    @ObservedObject var store: EditorStore
    @Environment(\.dismiss) private var dismiss

    @State private var volume: Double = 1.0
    @State private var muted: Bool = false

    var body: some View {
        NavigationStack {
            if let clip = store.selectedClip {
                Form {
                    Section("Audio") {
                        Toggle("Mute Clip", isOn: $muted)
                        VStack(alignment: .leading) {
                            Text("Volume: \(Int(volume * 100))%")
                            Slider(value: $volume, in: 0...2.0, step: 0.05)
                                .tint(ArArPalette.accent)
                        }
                    }
                    Section {
                        Button("Apply Changes") {
                            store.patchClip(clip.id, patch: ["volume": volume, "muted": muted])
                            dismiss()
                        }
                        .frame(maxWidth: .infinity)
                    }
                }
                .onAppear {
                    volume = clip.volume
                    muted = clip.muted
                }
                .navigationTitle("Clip Adjust")
                .toolbar {
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Done") { dismiss() }
                    }
                }
            }
        }
        .presentationDetents([.medium])
    }
}

// MARK: - 5. Export Video / GIF Sheet

struct ExportVideoSheet: View {
    @ObservedObject var store: EditorStore
    @Environment(\.dismiss) private var dismiss

    @State private var mode = 0 // 0 = MP4, 1 = GIF
    @State private var resolution: UInt32 = 1080
    @State private var fps: UInt32 = 30
    @State private var hevc = false
    @State private var exporting = false
    @State private var progress: Float = 0
    @State private var outputURL: URL?
    @State private var statusText: String?

    var body: some View {
        NavigationStack {
            Form {
                Section("Format") {
                    Picker("Output", selection: $mode) {
                        Text("MP4 Video").tag(0)
                        Text("Animated GIF").tag(1)
                    }
                    .pickerStyle(.segmented)
                    .disabled(exporting)

                    Picker("Resolution", selection: $resolution) {
                        Text("480p").tag(UInt32(480))
                        Text("720p").tag(UInt32(720))
                        Text("1080p Full HD").tag(UInt32(1080))
                        if mode == 0 {
                            Text("4K UHD").tag(UInt32(2160))
                        }
                    }
                    .disabled(exporting)

                    Picker("Frame Rate", selection: $fps) {
                        Text("24 fps").tag(UInt32(24))
                        Text("30 fps").tag(UInt32(30))
                        if mode == 0 {
                            Text("60 fps").tag(UInt32(60))
                        }
                    }
                    .disabled(exporting)

                    if mode == 0 {
                        Toggle("HEVC (H.265) Hardware Encode", isOn: $hevc)
                            .disabled(exporting)
                    }
                }

                if exporting {
                    Section("Rendering (VideoToolbox + Metal)") {
                        ProgressView(value: Double(progress))
                            .tint(ArArPalette.accent)
                        Text("\(Int(progress * 100))%")
                            .font(.caption.monospacedDigit())
                        Button("Cancel Export", role: .destructive) {
                            cancelExport()
                        }
                    }
                } else {
                    Section {
                        Button {
                            beginExport()
                        } label: {
                            Text(mode == 0 ? "Export MP4" : "Export GIF")
                                .font(.headline)
                                .frame(maxWidth: .infinity)
                        }
                    }
                }

                if let url = outputURL {
                    Section("Export Ready") {
                        ShareLink(item: url) {
                            Label("Share or Save to Files", systemImage: "square.and.arrow.up")
                        }
                        if mode == 0 {
                            Button {
                                saveVideoToPhotos(url)
                            } label: {
                                Label("Save to Photos Library", systemImage: "photo.on.rectangle")
                            }
                        }
                    }
                }

                if let statusText {
                    Section {
                        Text(statusText)
                            .font(.caption)
                            .foregroundStyle(ArArPalette.textSecondary)
                    }
                }
            }
            .navigationTitle("Export")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Close") { dismiss() }
                        .disabled(exporting)
                }
            }
        }
    }

    private func beginExport() {
        exporting = true
        progress = 0
        outputURL = nil
        statusText = nil

        let ext = mode == 0 ? "mp4" : "gif"
        let safeName = "ArArCut-\(Int(Date().timeIntervalSince1970)).\(ext)"
        guard let outPath = try? exportPath(name: safeName) else {
            exporting = false
            statusText = "Could not allocate export file path."
            return
        }

        let listener = SwiftExportListener(
            progress: { frac in
                Task { @MainActor in self.progress = frac }
            },
            done: { path in
                Task { @MainActor in
                    self.exporting = false
                    self.progress = 1.0
                    self.outputURL = URL(fileURLWithPath: path)
                    self.statusText = "Saved to \(URL(fileURLWithPath: path).lastPathComponent)"
                }
            },
            failed: { msg, cancelled in
                Task { @MainActor in
                    self.exporting = false
                    self.statusText = cancelled ? "Export cancelled." : msg
                }
            }
        )

        do {
            if mode == 0 {
                let dims = exportSize(width: store.timeline.width, height: store.timeline.height, resolution: resolution)
                let w = dims.first ?? 1080
                let h = dims.count > 1 ? dims[1] : 1920
                let br = suggestedBitrateKbps(width: w, height: h, fps: fps)
                try startExport(
                    session: store.session,
                    settings: ExportSettings(
                        resolution: resolution,
                        fps: fps,
                        bitrateKbps: br,
                        hevc: hevc,
                        output: outPath
                    ),
                    listener: listener
                )
            } else {
                try startGifExport(
                    session: store.session,
                    settings: GifSettings(
                        output: outPath,
                        shortSide: min(resolution, 720),
                        fps: min(fps, 24)
                    ),
                    listener: listener
                )
            }
        } catch {
            exporting = false
            statusText = error.localizedDescription
        }
    }

    private func saveVideoToPhotos(_ url: URL) {
        PHPhotoLibrary.requestAuthorization(for: .addOnly) { status in
            guard status == .authorized || status == .limited else {
                Task { @MainActor in statusText = "Photos permission denied." }
                return
            }
            PHPhotoLibrary.shared().performChanges({
                PHAssetChangeRequest.creationRequestForAssetFromVideo(atFileURL: url)
            }) { ok, err in
                Task { @MainActor in
                    statusText = ok ? "Saved to Photos Library!" : (err?.localizedDescription ?? "Failed saving to Photos.")
                }
            }
        }
    }
}

// MARK: - 6. Desktop NLE Interchange Export Sheet

struct NleExportSheet: View {
    @ObservedObject var store: EditorStore
    @Environment(\.dismiss) private var dismiss

    @State private var exportedFile: URL?
    @State private var summaryText: String?

    private let formats: [(NleFormat, String, String)] = [
        (.fcpxml, "Final Cut Pro / Resolve (.fcpxml)", "FCPXML v1.11 rational-time timeline for macOS Final Cut Pro & DaVinci Resolve."),
        (.premiereXml, "Premiere Pro / Resolve XML (.xml)", "Final Cut Pro 7 xmeml v5 timeline interchange."),
        (.otio, "OpenTimelineIO (.otio)", "ASWF/Pixar open JSON interchange for desktop pipelines."),
        (.edl, "CMX 3600 EDL (.edl)", "Universal edit decision list with SMPTE timecode."),
    ]

    var body: some View {
        NavigationStack {
            List {
                Section("Export Timeline Structure") {
                    ForEach(formats, id: \.1) { fmt, title, desc in
                        Button {
                            generate(fmt)
                        } label: {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(title).font(.subheadline.weight(.semibold))
                                Text(desc).font(.caption).foregroundStyle(ArArPalette.textSecondary)
                            }
                            .padding(.vertical, 2)
                        }
                    }
                }

                if let url = exportedFile {
                    Section("Ready to AirDrop / Share") {
                        ShareLink(item: url) {
                            Label("Share \(url.lastPathComponent)", systemImage: "square.and.arrow.up")
                        }
                        if let summaryText {
                            Text(summaryText).font(.caption).foregroundStyle(ArArPalette.textSecondary)
                        }
                    }
                }
            }
            .navigationTitle("Export to Desktop NLE")
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    private func generate(_ fmt: NleFormat) {
        let sess = store.session
        Task.detached(priority: .userInitiated) {
            do {
                let res = try sess.exportNle(format: fmt)
                let dest = FileManager.default.temporaryDirectory.appendingPathComponent(res.filename)
                try res.content.write(to: dest, atomically: true, encoding: .utf8)
                await MainActor.run {
                    self.exportedFile = dest
                    self.summaryText = "Generated \(res.filename) (\(res.clipCount) clips)"
                }
            } catch {
                await MainActor.run {
                    self.summaryText = error.localizedDescription
                }
            }
        }
    }
}
