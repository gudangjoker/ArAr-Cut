package com.ararcut.app.engine

import android.content.ContentResolver
import android.content.Context
import android.graphics.Bitmap
import android.net.Uri
import android.provider.OpenableColumns
import com.ararcut.app.core.mediaFolder
import java.io.File
import java.io.FileOutputStream

/** Picked files copied into a project, so the project keeps working whatever happens to the originals. */
object Media {
    /** Copies [uri] into the project's media folder and returns the copy. */
    fun importUri(context: Context, projectPath: String, uri: Uri): File {
        val resolver = context.contentResolver
        val dir = File(mediaFolder(projectPath)).apply { mkdirs() }
        val expectedSize = mediaSize(resolver, uri)
        if (expectedSize > 0L && dir.usableSpace < expectedSize + MIN_FREE_MARGIN_BYTES) {
            throw IllegalStateException("Not enough storage space to import this file.")
        }
        val name = safeName(displayName(resolver, uri))
        val out = uniqueFile(dir, name)
        try {
            resolver.openInputStream(uri)?.use { input -> FileOutputStream(out).use { input.copyTo(it, 1 shl 16) } }
                ?: throw IllegalStateException("Could not read the file.")
        } catch (e: Throwable) {
            out.delete()
            throw e
        }
        // kept so the picker can mark what is in this project already
        runCatching { File(projectPath, IMPORTED).appendText(uri.toString() + "\n") }
        return out
    }

    private const val MIN_FREE_MARGIN_BYTES = 50L * 1024 * 1024

    private fun mediaSize(resolver: ContentResolver, uri: Uri): Long {
        runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.SIZE), null, null, null)?.use { c ->
                if (c.moveToFirst() && !c.isNull(0)) return c.getLong(0)
            }
        }
        return -1L
    }

    /** The file in a project's folder listing the phone's media brought into it, a URI a line. */
    const val IMPORTED = "imported.txt"

    /** The media brought into the project at [projectPath], by URI. */
    fun imported(projectPath: String): Set<String> =
        runCatching { File(projectPath, IMPORTED).readLines().filter { it.isNotBlank() }.toSet() }.getOrDefault(emptySet())

    /** Saves a project's cover picture (a JPEG beside its document). */
    fun saveCover(projectPath: String, bitmap: Bitmap) {
        runCatching {
            val file = File(projectPath, "cover.jpg")
            val tmp = File(projectPath, "cover.jpg.tmp")
            FileOutputStream(tmp).use { bitmap.compress(Bitmap.CompressFormat.JPEG, 85, it) }
            tmp.renameTo(file)
        }
    }

    fun displayName(resolver: ContentResolver, uri: Uri): String {
        runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) return c.getString(0) ?: "media"
            }
        }
        return uri.lastPathSegment?.substringAfterLast('/') ?: "media"
    }

    fun safeName(name: String): String {
        val cleaned = name
            .replace(Regex("[^A-Za-z0-9._ -]"), "_")
            .trim()
            .trimStart('.')
            .replace(Regex("\\.{2,}"), ".")
            .take(80)
        return if (cleaned.isBlank() || cleaned == "." || cleaned == "..") "media" else cleaned
    }

    fun uniqueFile(dir: File, name: String): File {
        dir.mkdirs()
        var f = File(dir, name)
        var i = 1
        val stem = name.substringBeforeLast('.')
        val ext = name.substringAfterLast('.', "")
        while (f.exists()) {
            f = File(dir, if (ext.isEmpty()) "$stem ($i)" else "$stem ($i).$ext")
            i++
        }
        return f
    }
}
