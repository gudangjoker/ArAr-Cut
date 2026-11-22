// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 ArAr Cut contributors

import Foundation
import SwiftUI
import UIKit

// MARK: - Picture RGBA -> UIImage

extension Picture {
    func toUIImage() -> UIImage? {
        let w = Int(width)
        let h = Int(height)
        guard w > 0, h > 0, rgba.count >= w * h * 4 else { return nil }
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedLast.rawValue)
            .union(.byteOrder32Big)
        guard let provider = CGDataProvider(data: rgba as CFData),
              let cg = CGImage(
                  width: w,
                  height: h,
                  bitsPerComponent: 8,
                  bitsPerPixel: 32,
                  bytesPerRow: w * 4,
                  space: colorSpace,
                  bitmapInfo: bitmapInfo,
                  provider: provider,
                  decode: nil,
                  shouldInterpolate: true,
                  intent: .defaultIntent
              )
        else { return nil }
        return UIImage(cgImage: cg)
    }
}

func formatTimecode(_ us: Int64) -> String {
    let clamped = max(0, us)
    let totalSec = Double(clamped) / 1_000_000.0
    let mins = Int(totalSec) / 60
    let secs = Int(totalSec) % 60
    let tenths = Int((totalSec - floor(totalSec)) * 10.0)
    return String(format: "%02d:%02d.%d", mins, secs, tenths)
}

func formatBytes(_ bytes: Int64) -> String {
    let f = ByteCountFormatter()
    f.allowedUnits = [.useMB, .useGB, .useKB]
    f.countStyle = .file
    return f.string(fromByteCount: max(0, bytes))
}

// MARK: - AppStore (Engine lifecycle + Project library)

@MainActor
final class AppStore: ObservableObject {
    @Published var engineReady = false
    @Published var bootError: String?
    @Published var projects: [ProjectSummary] = []
    @Published var activeEditor: EditorStore?
    @Published var busyMessage: String?
    @Published var toast: String?

    func bootstrap() {
        guard !engineReady else { return }
        bootError = nil
        Task.detached(priority: .userInitiated) {
            do {
                let fm = FileManager.default
                let appSupport = try fm.url(
                    for: .applicationSupportDirectory,
                    in: .userDomainMask,
                    appropriateFor: nil,
                    create: true
                ).appendingPathComponent("ArArCut", isDirectory: true)
                let caches = try fm.url(
                    for: .cachesDirectory,
                    in: .userDomainMask,
                    appropriateFor: nil,
                    create: true
                ).appendingPathComponent("ArArCut", isDirectory: true)
                try fm.createDirectory(at: appSupport, withIntermediateDirectories: true)
                try fm.createDirectory(at: caches, withIntermediateDirectories: true)
                try startEngine(filesDir: appSupport.path, cacheDir: caches.path)
                let items = try listProjects()
                await MainActor.run {
                    self.projects = items
                    self.engineReady = true
                }
            } catch {
                await MainActor.run {
                    self.bootError = error.localizedDescription
                }
            }
        }
    }

    func reloadProjects() {
        Task.detached(priority: .userInitiated) {
            let items = (try? listProjects()) ?? []
            await MainActor.run {
                self.projects = items
            }
        }
    }

    func createAndOpenProject(name: String, width: UInt32 = 1080, height: UInt32 = 1920, fps: UInt32 = 30, initialMedia: [URL] = []) {
        busyMessage = "Creating project…"
        Task.detached(priority: .userInitiated) {
            do {
                let summary = try createProject(name: name, width: width, height: height, fps: fps)
                let session = try MobileSession.open(path: summary.path)
                if !initialMedia.isEmpty {
                    let mediaDir = try mediaFolder(path: summary.path)
                    var mediaIds: [String] = []
                    for src in initialMedia {
                        if let local = Self.copyIntoMediaDir(src, mediaDir: mediaDir) {
                            let info = try session.importMedia(path: local)
                            mediaIds.append(info.id)
                        }
                    }
                    if !mediaIds.isEmpty {
                        _ = try session.addToMain(mediaIds: mediaIds, atUs: 0)
                        try? session.save()
                    }
                }
                let editor = await MainActor.run { EditorStore(session: session, projectPath: summary.path) }
                await editor.refreshFrame(motion: .still)
                let refreshed = (try? listProjects()) ?? []
                await MainActor.run {
                    self.projects = refreshed
                    self.busyMessage = nil
                    self.activeEditor = editor
                }
            } catch {
                await MainActor.run {
                    self.busyMessage = nil
                    self.toast = error.localizedDescription
                }
            }
        }
    }

    func openProject(_ summary: ProjectSummary) {
        busyMessage = "Opening \(summary.name)…"
        Task.detached(priority: .userInitiated) {
            do {
                let session = try MobileSession.open(path: summary.path)
                let editor = await MainActor.run { EditorStore(session: session, projectPath: summary.path) }
                await editor.refreshFrame(motion: .still)
                await MainActor.run {
                    self.busyMessage = nil
                    self.activeEditor = editor
                }
            } catch {
                await MainActor.run {
                    self.busyMessage = nil
                    self.toast = error.localizedDescription
                }
            }
        }
    }

    func closeEditor() {
        if let ed = activeEditor {
            ed.stopPlayback()
            let session = ed.session
            Task.detached(priority: .utility) {
                try? session.save()
                clearPlayback()
                trimMemory()
                let refreshed = (try? listProjects()) ?? []
                await MainActor.run {
                    self.projects = refreshed
                }
            }
        }
        activeEditor = nil
    }

    func rename(_ project: ProjectSummary, to newName: String) {
        let trimmed = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        Task.detached {
            _ = try? renameProject(path: project.path, name: trimmed)
            let items = (try? listProjects()) ?? []
            await MainActor.run { self.projects = items }
        }
    }

    func duplicate(_ project: ProjectSummary) {
        Task.detached {
            _ = try? duplicateProject(path: project.path, name: "\(project.name) Copy")
            let items = (try? listProjects()) ?? []
            await MainActor.run { self.projects = items }
        }
    }

    func delete(_ project: ProjectSummary) {
        Task.detached {
            try? deleteProject(path: project.path)
            let items = (try? listProjects()) ?? []
            await MainActor.run { self.projects = items }
        }
    }

    nonisolated static func copyIntoMediaDir(_ url: URL, mediaDir: String) -> String? {
        let accessed = url.startAccessingSecurityScopedResource()
        defer { if accessed { url.stopAccessingSecurityScopedResource() } }
        let fm = FileManager.default
        try? fm.createDirectory(atPath: mediaDir, withIntermediateDirectories: true)
        let ext = url.pathExtension.isEmpty ? "mov" : url.pathExtension
        let stem = url.deletingPathExtension().lastPathComponent
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "..", with: "_")
        let safeStem = stem.isEmpty ? "clip" : String(stem.prefix(48))
        let dest = URL(fileURLWithPath: mediaDir)
            .appendingPathComponent("\(safeStem)-\(UUID().uuidString.prefix(8)).\(ext)")
        do {
            try fm.copyItem(at: url, to: dest)
            return dest.path
        } catch {
            return nil
        }
    }
}

// MARK: - Playback & Export UniFFI Callback Bridges

private final class SwiftPlaybackListener: PlaybackListener, @unchecked Sendable {
    private let onErr: @Sendable (String) -> Void
    init(_ onErr: @escaping @Sendable (String) -> Void) { self.onErr = onErr }
    func onError(message: String) { onErr(message) }
}

final class SwiftExportListener: ExportListener, @unchecked Sendable {
    private let progress: @Sendable (Float) -> Void
    private let done: @Sendable (String) -> Void
    private let failed: @Sendable (String, Bool) -> Void

    init(
        progress: @escaping @Sendable (Float) -> Void,
        done: @escaping @Sendable (String) -> Void,
        failed: @escaping @Sendable (String, Bool) -> Void
    ) {
        self.progress = progress
        self.done = done
        self.failed = failed
    }

    func onProgress(fraction: Float) { progress(fraction) }
    func onDone(path: String) { done(path) }
    func onFailed(message: String, cancelled: Bool) { failed(message, cancelled) }
}

// MARK: - EditorStore

@MainActor
final class EditorStore: ObservableObject {
    let session: MobileSession
    let projectPath: String

    @Published var timeline: TimelineView
    @Published var playheadUs: Int64 = 0
    @Published var isPlayingNow = false
    @Published var selectedClipId: String?
    @Published var previewImage: UIImage?
    @Published var lastPicture: Picture?
    @Published var scopeKind: VideoScopeKind?
    @Published var scopeImage: UIImage?
    @Published var message: String?
    @Published var isWorking = false
    @Published var pixelsPerSecond: CGFloat = 68

    private var playbackTask: Task<Void, Never>?
    private var frameTask: Task<Void, Never>?
    private var scopeTask: Task<Void, Never>?

    init(session: MobileSession, projectPath: String) {
        self.session = session
        self.projectPath = projectPath
        let view = session.view()
        self.timeline = view
        self.selectedClipId = view.rows.first(where: { $0.kind == .main })?.clips.first?.id
        setPlaybackListener(listener: SwiftPlaybackListener { [weak self] msg in
            Task { @MainActor in
                self?.isPlayingNow = false
                self?.message = msg
            }
        })
    }

    var selectedClip: ClipView? {
        guard let id = selectedClipId else { return nil }
        for row in timeline.rows {
            if let clip = row.clips.first(where: { $0.id == id }) {
                return clip
            }
        }
        return nil
    }

    var selectedRow: RowView? {
        guard let id = selectedClipId else { return nil }
        return timeline.rows.first(where: { row in row.clips.contains(where: { $0.id == id }) })
    }

    func applyView(_ view: TimelineView, select: String? = nil) {
        timeline = view
        if let select {
            selectedClipId = select
        } else if !view.createdId.isEmpty {
            selectedClipId = view.createdId
        } else if let cur = selectedClipId,
                  !view.rows.contains(where: { r in r.clips.contains(where: { $0.id == cur }) }) {
            selectedClipId = nil
        }
        if playheadUs > view.durationUs {
            playheadUs = max(0, view.durationUs)
        }
        Task { await refreshFrame(motion: .still) }
        let sess = session
        Task.detached(priority: .utility) { try? sess.save() }
    }

    func runEdit(_ action: @escaping @Sendable (MobileSession) throws -> TimelineView) {
        let sess = session
        isWorking = true
        Task.detached(priority: .userInitiated) {
            do {
                let next = try action(sess)
                await MainActor.run {
                    self.isWorking = false
                    self.applyView(next)
                }
            } catch {
                await MainActor.run {
                    self.isWorking = false
                    self.message = error.localizedDescription
                }
            }
        }
    }

    func undo() {
        let sess = session
        Task.detached(priority: .userInitiated) {
            let next = sess.undo()
            await MainActor.run { self.applyView(next) }
        }
    }

    func redo() {
        let sess = session
        Task.detached(priority: .userInitiated) {
            let next = sess.redo()
            await MainActor.run { self.applyView(next) }
        }
    }

    func importFiles(_ urls: [URL], asOverlay: Bool = false, asAudio: Bool = false) {
        guard !urls.isEmpty else { return }
        let sess = session
        let projPath = projectPath
        let at = playheadUs
        isWorking = true
        Task.detached(priority: .userInitiated) {
            do {
                let mediaDir = try mediaFolder(path: projPath)
                var latestView: TimelineView?
                var mainIds: [String] = []
                for url in urls {
                    guard let local = AppStore.copyIntoMediaDir(url, mediaDir: mediaDir) else { continue }
                    if asAudio {
                        let info = try sess.importSound(path: local)
                        latestView = try sess.addAudio(mediaId: info.id, atUs: at)
                    } else if asOverlay {
                        let info = try sess.importMedia(path: local)
                        latestView = try sess.addOverlay(mediaId: info.id, atUs: at)
                    } else {
                        let info = try sess.importMedia(path: local)
                        mainIds.append(info.id)
                    }
                }
                if !mainIds.isEmpty {
                    latestView = try sess.addToMain(mediaIds: mainIds, atUs: at)
                }
                let finalView = latestView
                await MainActor.run {
                    self.isWorking = false
                    if let v = finalView {
                        self.applyView(v)
                    } else {
                        self.message = "Could not import selected media."
                    }
                }
            } catch {
                await MainActor.run {
                    self.isWorking = false
                    self.message = error.localizedDescription
                }
            }
        }
    }

    func patchClip(_ clipId: String, patch: [String: Any]) {
        let cmd: [String: Any] = ["op": "updateClip", "clipId": clipId, "patch": patch]
        guard let data = try? JSONSerialization.data(withJSONObject: cmd),
              let json = String(data: data, encoding: .utf8)
        else { return }
        runEdit { try $0.apply(command: json, gesture: nil) }
    }

    func togglePlay() {
        if isPlayingNow {
            stopPlayback()
        } else {
            startPlayback()
        }
    }

    func startPlayback() {
        guard timeline.durationUs > 0 else { return }
        if playheadUs >= timeline.durationUs { playheadUs = 0 }
        let startAt = playheadUs
        isPlayingNow = true
        Task.detached(priority: .userInitiated) {
            do {
                try play(atUs: startAt)
            } catch {
                await MainActor.run {
                    self.isPlayingNow = false
                    self.message = error.localizedDescription
                }
            }
        }
        playbackTask?.cancel()
        playbackTask = Task { [weak self] in
            while let self, !Task.isCancelled, self.isPlayingNow {
                let pos = positionUs()
                let playing = isPlaying()
                self.playheadUs = min(pos, self.timeline.durationUs)
                await self.refreshFrame(motion: .playing)
                if !playing || pos >= self.timeline.durationUs {
                    self.isPlayingNow = false
                    await self.refreshFrame(motion: .still)
                    break
                }
                try? await Task.sleep(nanoseconds: 33_000_000) // ~30 fps UI sync
            }
        }
    }

    func stopPlayback() {
        isPlayingNow = false
        playbackTask?.cancel()
        playbackTask = nil
        ArArCut.pause()
        Task { await refreshFrame(motion: .still) }
    }

    func scrub(to us: Int64, scrubbing: Bool) {
        let clamped = max(0, min(us, timeline.durationUs))
        playheadUs = clamped
        if isPlayingNow { stopPlayback() }
        seek(atUs: clamped)
        Task { await refreshFrame(motion: scrubbing ? .scrubbing : .still) }
    }

    func refreshFrame(motion: Motion) async {
        let sess = session
        let at = playheadUs
        let aspect = Double(max(1, timeline.width)) / Double(max(1, timeline.height))
        let maxSide: Double = motion == .scrubbing ? 480 : 720
        let w: UInt32
        let h: UInt32
        if aspect >= 1.0 {
            w = UInt32(maxSide)
            h = max(2, UInt32((maxSide / aspect).rounded()) & ~1)
        } else {
            h = UInt32(maxSide)
            w = max(2, UInt32((maxSide * aspect).rounded()) & ~1)
        }
        frameTask?.cancel()
        let task = Task.detached(priority: .userInitiated) { () -> (Picture, UIImage?)? in
            guard let pic = try? sess.frame(timeUs: at, width: w, height: h, motion: motion) else {
                return nil
            }
            return (pic, pic.toUIImage())
        }
        frameTask = Task {
            if let (pic, img) = await task.value {
                self.lastPicture = pic
                if let img { self.previewImage = img }
                self.refreshScope()
            }
        }
        await frameTask?.value
    }

    func refreshScope() {
        guard let kind = scopeKind, let pic = lastPicture else {
            scopeImage = nil
            return
        }
        scopeTask?.cancel()
        scopeTask = Task.detached(priority: .utility) {
            guard let rendered = try? renderVideoScope(rgba: pic.rgba, width: pic.width, height: pic.height, kind: kind),
                  let img = rendered.toUIImage()
            else { return }
            await MainActor.run {
                if self.scopeKind == kind {
                    self.scopeImage = img
                }
            }
        }
    }
}
