package com.ararcut.app.ui.editor.panels

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ararcut.app.core.AudioSyncReport
import com.ararcut.app.core.LoudnessReport
import com.ararcut.app.core.NleExportResult
import com.ararcut.app.core.NleFormat
import com.ararcut.app.core.RowKind
import com.ararcut.app.core.VideoScopeKind
import com.ararcut.app.core.analyzeClipLoudness
import com.ararcut.app.core.cameraLogProfiles
import com.ararcut.app.core.installCameraLogLook
import com.ararcut.app.core.measureAudioSync
import com.ararcut.app.core.proAudioPresets
import com.ararcut.app.core.renderVideoScope
import com.ararcut.app.engine.Threads
import com.ararcut.app.engine.toBitmap
import com.ararcut.app.engine.userMessage
import com.ararcut.app.ui.Glyph
import com.ararcut.app.ui.LucideIcons
import com.ararcut.app.ui.Palette
import com.ararcut.app.ui.Type
import com.ararcut.app.ui.editor.EditorViewModel
import java.nio.ByteBuffer
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject

/**
 * Broadcast Audio DSP panel powered by FilmCraft's `filmcraft-audio-dsp`:
 * - EBU R128 / ITU-R BS.1770-4 loudness & true-peak meter
 * - Curated pro mastering/restoration chains with target LUFS selector
 * - GCC-PHAT sample-accurate audio sync to the main track.
 */
@Composable
fun ProAudioPanel(vm: EditorViewModel) {
    val clip = vm.selectedClip
    val row = vm.selectedRow
    val session = vm.session
    val presets = remember { proAudioPresets() }
    val scope = rememberCoroutineScope()

    var targetLufs by remember { mutableDoubleStateOf(-14.0) }
    var loudness by remember(clip?.id) { mutableStateOf<LoudnessReport?>(null) }
    var syncReport by remember(clip?.id) { mutableStateOf<AudioSyncReport?>(null) }
    var hasProcessed by remember(clip?.id) { mutableStateOf(false) }
    var working by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(clip?.id, vm.timeline?.revision) {
        if (clip != null && session != null) {
            hasProcessed = withContext(Threads.commands) { runCatching { session.hasProAudio(clip.id) }.getOrDefault(false) }
        }
    }

    PanelFrame("Pro Audio DSP", vm, tall = true) {
        if (clip == null || session == null) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("Select a video or audio clip first", style = Type.Small)
            }
            return@PanelFrame
        }
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            // 1. EBU R128 Loudness Meter card
            Column(
                Modifier
                    .fillMaxWidth()
                    .clip(RoundedCornerShape(10.dp))
                    .background(Color(0xFF191B20))
                    .padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Column(Modifier.weight(1f)) {
                        Text("EBU R128 Loudness Meter", style = Type.BodyStrong.copy(fontSize = 14.sp))
                        Text("ITU-R BS.1770-4 integrated & 4x true peak", style = Type.Small)
                    }
                    ActionChip(
                        label = if (working == "meter") "Scanning…" else "Measure",
                        accent = true,
                        enabled = working == null,
                    ) {
                        scope.launch {
                            working = "meter"
                            val res = withContext(Threads.commands) { runCatching { analyzeClipLoudness(clip.path) } }
                            working = null
                            res.onSuccess { loudness = it }.onFailure { vm.message = it.userMessage }
                        }
                    }
                }
                loudness?.let { m ->
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        MetricCell("Integrated", "%.1f LUFS".format(m.integratedLufs))
                        MetricCell("True Peak", "%.1f dBTP".format(m.truePeakDbtp))
                        MetricCell("Range", "%.1f LU".format(m.loudnessRangeLu))
                        MetricCell("Short-Term", "%.1f LUFS".format(m.maxShortTermLufs))
                    }
                }
            }

            // 2. Target LUFS selector + Revert button
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("Target:", style = Type.Small.copy(color = Palette.Text))
                listOf(-14.0 to "-14 Social", -16.0 to "-16 Voice", -23.0 to "-23 EBU").forEach { (lufs, label) ->
                    val active = targetLufs == lufs
                    Box(
                        Modifier
                            .clip(RoundedCornerShape(6.dp))
                            .background(if (active) Palette.Accent else Color(0xFF23262E))
                            .clickable(enabled = working == null) { targetLufs = lufs }
                            .padding(horizontal = 10.dp, vertical = 6.dp),
                    ) {
                        Text(
                            label,
                            style = Type.Small.copy(
                                color = if (active) Palette.OnAccent else Palette.Text,
                                fontWeight = if (active) FontWeight.SemiBold else FontWeight.Normal,
                            ),
                        )
                    }
                }
                Spacer(Modifier.weight(1f))
                if (hasProcessed) {
                    ActionChip("Revert", accent = false, enabled = working == null) {
                        scope.launch {
                            working = "revert"
                            val res = withContext(Threads.commands) { runCatching { session.revertProAudio(clip.id) } }
                            working = null
                            res.onSuccess { view ->
                                vm.applyView(view)
                                hasProcessed = false
                                loudness = null
                                vm.message = "Reverted to original audio"
                            }.onFailure { vm.message = it.userMessage }
                        }
                    }
                }
            }

            // 3. Mastering & Restoration Presets
            Text("Mastering & Restoration Chains", style = Type.BodyStrong.copy(fontSize = 13.sp, color = Palette.TextSecondary))
            presets.forEach { preset ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(10.dp))
                        .background(Color(0xFF1E2028))
                        .clickable(enabled = working == null) {
                            scope.launch {
                                working = preset.id
                                val res = withContext(Threads.commands) {
                                    runCatching { session.applyProAudio(clip.id, preset.id, targetLufs) }
                                }
                                working = null
                                res.onSuccess { view ->
                                    vm.applyView(view)
                                    hasProcessed = true
                                    loudness = null
                                    vm.message = "Applied ${preset.name}"
                                }.onFailure { vm.message = it.userMessage }
                            }
                        }
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(preset.name, style = Type.BodyStrong.copy(fontSize = 14.sp))
                        Spacer(Modifier.height(2.dp))
                        Text(preset.summary, style = Type.Small, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                    Spacer(Modifier.width(10.dp))
                    if (working == preset.id) {
                        CircularProgressIndicator(color = Palette.Accent, modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    } else {
                        Glyph(LucideIcons.AudioWaveform, Palette.Accent, size = 18.dp)
                    }
                }
            }

            // 4. GCC-PHAT Auto Audio Sync (for audio or overlay clips against the main track)
            val mainRef = vm.timeline?.rows?.firstOrNull { it.kind == RowKind.MAIN }?.clips?.firstOrNull { it.audible }
            val currentRow = vm.rowOf(clip.id)
            if (mainRef != null && mainRef.id != clip.id && row != RowKind.MAIN && currentRow != null) {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(10.dp))
                        .background(Color(0xFF191B20))
                        .padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text("GCC-PHAT Auto Audio Sync", style = Type.BodyStrong.copy(fontSize = 14.sp))
                            Text("Sample-accurate alignment to main track audio", style = Type.Small)
                        }
                        ActionChip(
                            label = if (working == "sync") "Syncing…" else "Sync",
                            accent = true,
                            enabled = working == null,
                        ) {
                            scope.launch {
                                working = "sync"
                                val res = withContext(Threads.commands) {
                                    runCatching { measureAudioSync(mainRef.path, clip.path) }
                                }
                                working = null
                                res.onSuccess { report ->
                                    syncReport = report
                                    val alignedStart = (mainRef.startUs + report.offsetUs).coerceAtLeast(0L)
                                    vm.edit { moveClip(clip.id, currentRow.trackId, alignedStart) }
                                    val ms = report.offsetUs / 1000.0
                                    vm.message = if (report.reliable) {
                                        "Synced (offset %+.1f ms, conf %.1f)".format(ms, report.confidence)
                                    } else {
                                        "Aligned (low confidence %.1f, offset %+.1f ms)".format(report.confidence, ms)
                                    }
                                }.onFailure { vm.message = it.userMessage }
                            }
                        }
                    }
                    syncReport?.let { s ->
                        Text(
                            "Offset: %+.1f ms · Confidence: %.1f · %s".format(
                                s.offsetUs / 1000.0,
                                s.confidence,
                                if (s.reliable) "High reliability" else "Check alignment",
                            ),
                            style = Type.Small.copy(color = if (s.reliable) Palette.Accent else Palette.TextSecondary),
                        )
                    }
                }
            }
        }
    }
}

/**
 * Camera Log -> Rec.709 GPU LUT panel powered by FilmCraft's `filmcraft-color`,
 * with quick access to Live Video Scopes (`filmcraft-scopes`).
 */
@Composable
fun CameraLogPanel(vm: EditorViewModel) {
    val profiles = remember { cameraLogProfiles() }
    val scope = rememberCoroutineScope()
    var applying by remember { mutableStateOf<String?>(null) }

    PanelFrame(
        title = "Camera Log → Rec.709",
        vm = vm,
        tall = true,
        left = {
            Row(
                Modifier
                    .clip(RoundedCornerShape(6.dp))
                    .background(if (vm.scopeKind != null) Palette.Accent else Color(0xFF252830))
                    .clickable {
                        vm.scopeKind = if (vm.scopeKind == null) VideoScopeKind.WAVEFORM_RGB else null
                    }
                    .padding(horizontal = 10.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Glyph(
                    LucideIcons.Activity,
                    if (vm.scopeKind != null) Palette.OnAccent else Palette.Text,
                    size = 14.dp,
                )
                Spacer(Modifier.width(5.dp))
                Text(
                    "Scopes",
                    style = Type.Small.copy(
                        color = if (vm.scopeKind != null) Palette.OnAccent else Palette.Text,
                        fontWeight = FontWeight.SemiBold,
                    ),
                )
            }
        },
    ) {
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Bakes a 33³ 3D LUT with BT.2390 highlight roll-off & ACES gamut compression so it runs on the GPU at 60 fps.",
                style = Type.Small,
            )
            profiles.forEach { profile ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(10.dp))
                        .background(Color(0xFF1E2028))
                        .clickable(enabled = applying == null) {
                            scope.launch {
                                applying = profile.id
                                val res = withContext(Threads.commands) {
                                    runCatching { installCameraLogLook(profile.id) }
                                }
                                applying = null
                                res.onSuccess { lookId ->
                                    PackageCache.changed("filter")
                                    val clip = vm.selectedClip
                                    if (clip != null && (clip.kind == "video" || clip.kind == "image")) {
                                        val chain = vm.clipJson(clip.id)?.optJSONArray("videoEffects") ?: JSONArray()
                                        val next = JSONArray()
                                        for (i in 0 until chain.length()) {
                                            val link = chain.getJSONObject(i)
                                            if (link.optString("id") != lookId) next.put(link)
                                        }
                                        next.put(JSONObject().put("id", lookId).put("params", JSONObject()))
                                        vm.patch(clip.id, JSONObject().put("videoEffects", next))
                                    } else {
                                        vm.addLayer(lookId, profile.name)
                                    }
                                    vm.message = "Applied ${profile.camera} ${profile.name}"
                                }.onFailure { vm.message = it.userMessage }
                            }
                        }
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier
                            .clip(RoundedCornerShape(5.dp))
                            .background(Color(0xFF2B2F3A))
                            .padding(horizontal = 8.dp, vertical = 4.dp),
                    ) {
                        Text(profile.camera, style = Type.Small.copy(color = Palette.Accent, fontWeight = FontWeight.SemiBold))
                    }
                    Spacer(Modifier.width(10.dp))
                    Column(Modifier.weight(1f)) {
                        Text(profile.name, style = Type.BodyStrong.copy(fontSize = 14.sp))
                        Text("Gamut: ${profile.gamut} → Rec.709", style = Type.Small)
                    }
                    if (applying == profile.id) {
                        CircularProgressIndicator(color = Palette.Accent, modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    } else {
                        Glyph(LucideIcons.Film, Palette.TextSecondary, size = 18.dp)
                    }
                }
            }
        }
    }
}

/**
 * Floating live Video Scopes HUD overlay over the editor preview, powered by `filmcraft-scopes`.
 */
@Composable
fun VideoScopesHud(vm: EditorViewModel, frame: ImageBitmap?, modifier: Modifier = Modifier) {
    val kind = vm.scopeKind ?: return
    var scopeImage by remember { mutableStateOf<ImageBitmap?>(null) }

    LaunchedEffect(kind, frame) {
        val src = vm.lastFrame ?: return@LaunchedEffect
        if (src.isRecycled || src.width <= 0 || src.height <= 0) return@LaunchedEffect
        val w = src.width
        val h = src.height
        val bytes = runCatching {
            val buf = ByteBuffer.allocate(w * h * 4)
            src.copyPixelsToBuffer(buf)
            buf.array()
        }.getOrNull() ?: return@LaunchedEffect
        scopeImage = withContext(Threads.thumbnails) {
            runCatching { renderVideoScope(bytes, w.toUInt(), h.toUInt(), kind).toBitmap()?.asImageBitmap() }.getOrNull()
        }
    }

    Column(
        modifier
            .width(196.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(Color(0xD90A0C10))
            .border(1.dp, Color(0x33FFFFFF), RoundedCornerShape(10.dp))
            .padding(6.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            listOf(
                VideoScopeKind.WAVEFORM_RGB to "WF",
                VideoScopeKind.WAVEFORM_LUMA to "Y",
                VideoScopeKind.PARADE_RGB to "RGB",
                VideoScopeKind.VECTORSCOPE_YUV to "VEC",
                VideoScopeKind.HISTOGRAM to "HIST",
            ).forEach { (mode, label) ->
                val active = mode == kind
                Box(
                    Modifier
                        .clip(RoundedCornerShape(4.dp))
                        .background(if (active) Palette.Accent else Color(0x26FFFFFF))
                        .clickable { vm.scopeKind = mode }
                        .padding(horizontal = 6.dp, vertical = 2.dp),
                ) {
                    Text(
                        label,
                        style = Type.Small.copy(
                            fontSize = 10.sp,
                            color = if (active) Palette.OnAccent else Palette.Text,
                            fontWeight = FontWeight.SemiBold,
                        ),
                    )
                }
            }
            Box(
                Modifier
                    .clip(RoundedCornerShape(4.dp))
                    .background(Color(0x26FFFFFF))
                    .clickable { vm.scopeKind = null }
                    .padding(horizontal = 5.dp, vertical = 2.dp),
            ) {
                Text("×", style = Type.Small.copy(fontSize = 10.sp, color = Palette.Text))
            }
        }
        Box(
            Modifier
                .fillMaxWidth()
                .height(if (kind == VideoScopeKind.VECTORSCOPE_YUV) 124.dp else 92.dp)
                .clip(RoundedCornerShape(6.dp))
                .background(Color.Black),
            contentAlignment = Alignment.Center,
        ) {
            scopeImage?.let { img ->
                Image(
                    bitmap = img,
                    contentDescription = "Video scope",
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxSize().padding(2.dp),
                )
            } ?: Text("Waiting for frame…", style = Type.Small.copy(fontSize = 10.sp))
        }
    }
}

@Composable
private fun MetricCell(label: String, value: String) {
    Column(horizontalAlignment = Alignment.Start) {
        Text(label, style = Type.Small.copy(fontSize = 10.sp))
        Text(value, style = Type.BodyStrong.copy(fontSize = 12.sp, color = Palette.Accent))
    }
}

@Composable
private fun ActionChip(label: String, accent: Boolean, enabled: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .clip(RoundedCornerShape(6.dp))
            .background(if (accent) Palette.Accent else Color(0xFF2B2E38))
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 12.dp, vertical = 6.dp),
    ) {
        Text(
            label,
            style = Type.Small.copy(
                color = if (accent) Palette.OnAccent else Palette.Text,
                fontWeight = FontWeight.SemiBold,
            ),
        )
    }
}

/**
 * Classical MFCC speaker diarization panel (adapted from FilmCraft's `diarize.rs`):
 * clusters caption clips by voice timbre with zero AI model downloads and prefixes
 * captions with `"Speaker 1: ..."`, `"Speaker 2: ..."`, etc. in one undo step.
 */
@Composable
fun SpeakerDiarizePanel(vm: EditorViewModel) {
    val session = vm.session
    val scope = rememberCoroutineScope()
    var maxSpeakers by remember { mutableIntStateOf(4) }
    var working by remember { mutableStateOf(false) }

    PanelFrame("Label Speakers", vm, height = 240.dp) {
        if (session == null) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("Project not ready", style = Type.Small)
            }
            return@PanelFrame
        }
        Column(
            Modifier
                .fillMaxSize()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text(
                "Detects different voices across your caption clips using on-device MFCC clustering (no model download required) and labels them Speaker 1, Speaker 2…",
                style = Type.Small,
            )
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Text("Max speakers:", style = Type.Small.copy(color = Palette.Text))
                listOf(2, 3, 4, 6).forEach { count ->
                    val active = maxSpeakers == count
                    Box(
                        Modifier
                            .clip(RoundedCornerShape(6.dp))
                            .background(if (active) Palette.Accent else Color(0xFF23262E))
                            .clickable(enabled = !working) { maxSpeakers = count }
                            .padding(horizontal = 12.dp, vertical = 6.dp),
                    ) {
                        Text(
                            "$count",
                            style = Type.Small.copy(
                                color = if (active) Palette.OnAccent else Palette.Text,
                                fontWeight = FontWeight.SemiBold,
                            ),
                        )
                    }
                }
            }
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(42.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .background(Palette.Accent)
                    .clickable(enabled = !working) {
                        scope.launch {
                            working = true
                            val res = withContext(Threads.commands) {
                                runCatching { session.labelCaptionSpeakers(maxSpeakers.toUInt()) }
                            }
                            working = false
                            res.onSuccess { report ->
                                vm.applyView(report.view)
                                vm.message = "Found ${report.speakersFound} speaker(s) across ${report.captionsLabelled} caption(s)"
                                vm.panel = null
                            }.onFailure { vm.message = it.userMessage }
                        }
                    },
                contentAlignment = Alignment.Center,
            ) {
                if (working) {
                    CircularProgressIndicator(color = Palette.OnAccent, modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                } else {
                    Text(
                        "Auto-Label Caption Speakers",
                        style = Type.BodyStrong.copy(color = Palette.OnAccent, fontSize = 14.sp),
                    )
                }
            }
        }
    }
}

/**
 * Desktop NLE Interchange Export panel: exports the active timeline to Adobe Premiere Pro XML
 * (`xmeml`), DaVinci Resolve / Final Cut Pro (`fcpxml`), OpenTimelineIO (`.otio`), or CMX 3600 EDL.
 */
@Composable
fun NleExportPanel(vm: EditorViewModel) {
    val session = vm.session
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var pending by remember { mutableStateOf<NleExportResult?>(null) }
    var working by remember { mutableStateOf<NleFormat?>(null) }

    val saveLauncher = rememberLauncherForActivityResult(ActivityResultContracts.CreateDocument("*/*")) { uri ->
        val doc = pending
        pending = null
        if (uri == null || doc == null) return@rememberLauncherForActivityResult
        scope.launch {
            val res = withContext(Threads.commands) {
                runCatching {
                    context.contentResolver.openOutputStream(uri, "wt")?.use {
                        it.write(doc.content.toByteArray(Charsets.UTF_8))
                    } ?: error("Could not write file.")
                }
            }
            res.onSuccess {
                vm.message = "Exported ${doc.filename} (${doc.clipCount} clips)"
            }.onFailure { vm.message = it.userMessage }
        }
    }

    val formats = remember {
        listOf(
            Triple(NleFormat.PREMIERE_XML, "Premiere Pro / Resolve XML (.xml)", "Final Cut Pro 7 xmeml v5 — native for Adobe Premiere Pro & DaVinci Resolve."),
            Triple(NleFormat.FCPXML, "Final Cut Pro / Resolve (.fcpxml)", "FCPXML v1.11 rational-time timeline with asset references."),
            Triple(NleFormat.OTIO, "OpenTimelineIO (.otio)", "ASWF/Pixar open JSON interchange for desktop pipelines."),
            Triple(NleFormat.EDL, "CMX 3600 EDL (.edl)", "Universal edit decision list with SMPTE timecode & speed events."),
        )
    }

    PanelFrame("Export to Desktop NLE", vm, tall = true) {
        if (session == null) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                Text("Project not ready", style = Type.Small)
            }
            return@PanelFrame
        }
        Column(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Export your timeline structure to continue editing in desktop NLEs.",
                style = Type.Small,
            )
            formats.forEach { (fmt, title, desc) ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(10.dp))
                        .background(Color(0xFF1E2028))
                        .clickable(enabled = working == null) {
                            scope.launch {
                                working = fmt
                                val res = withContext(Threads.commands) { runCatching { session.exportNle(fmt) } }
                                working = null
                                res.onSuccess { doc ->
                                    pending = doc
                                    saveLauncher.launch(doc.filename)
                                }.onFailure { vm.message = it.userMessage }
                            }
                        }
                        .padding(horizontal = 12.dp, vertical = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Column(Modifier.weight(1f)) {
                        Text(title, style = Type.BodyStrong.copy(fontSize = 14.sp))
                        Spacer(Modifier.height(2.dp))
                        Text(desc, style = Type.Small, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                    Spacer(Modifier.width(10.dp))
                    if (working == fmt) {
                        CircularProgressIndicator(color = Palette.Accent, modifier = Modifier.size(18.dp), strokeWidth = 2.dp)
                    } else {
                        Glyph(LucideIcons.ExternalLink, Palette.Accent, size = 18.dp)
                    }
                }
            }
        }
    }
}
