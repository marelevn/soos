package io.github.marelevn.soos

import org.json.JSONArray
import org.json.JSONObject

/** One line's result as soos-core's `result_cells` shows it. */
data class Cell(val text: String, val copy: String, val error: String?, val total: Boolean)

/** A converter's value (`ok`) or why it's invalid (`error`). */
data class ConverterStatus(val ok: String?, val error: String?)

/** A recalculated document: a status per converter, a cell per line. */
data class Calculated(val converters: List<ConverterStatus>, val lines: List<Cell?>)

/** The kinds soos-core's highlighter gives, in the order of its `kind_code`. */
enum class TokenKind { Comment, Header, Label, Keyword, ConversionWord }

/** A highlighted stretch of the document, in UTF-16 units. */
data class Span(val start: Int, val end: Int, val kind: TokenKind)

/**
 * soos-core, through the JNI crate `soos-android`. Every call that takes
 * time (`recalc`, `refreshRates`) blocks, so they're called off the main
 * thread; see [AppState].
 */
object Engine {
    init {
        System.loadLibrary("soos_android")
    }

    @JvmStatic external fun init(ratesPath: String)

    /** Cancels any recalculation older than `generation`. */
    @JvmStatic external fun cancelOlderThan(generation: Long)

    @JvmStatic private external fun recalc(
        generation: Long,
        text: String,
        convertersJson: String,
        highPrecision: Boolean,
        decimalComma: Boolean,
    ): String?

    @JvmStatic private external fun highlight(text: String): IntArray

    /** Downloads the exchange rates if the saved ones are stale. */
    @JvmStatic external fun refreshRates()

    /** How old the saved rates are in seconds, or -1 if there are none. */
    @JvmStatic external fun ratesAgeSeconds(): Long

    /** `null` if a newer recalculation was asked for meanwhile. */
    fun calculate(
        generation: Long,
        text: String,
        converters: List<ConverterRow>,
        highPrecision: Boolean,
        decimalComma: Boolean,
    ): Calculated? {
        val json = JSONArray(converters.map { it.toJson() }).toString()
        return recalc(generation, text, json, highPrecision, decimalComma)?.let(::parseCalculated)
    }

    fun spans(text: String): List<Span> {
        val flat = highlight(text)
        return List(flat.size / 3) {
            Span(flat[it * 3], flat[it * 3 + 1], TokenKind.entries[flat[it * 3 + 2]])
        }
    }
}

fun parseCalculated(json: String): Calculated {
    val root = JSONObject(json)
    val converters = root.getJSONArray("converters").let { array ->
        List(array.length()) {
            val status = array.getJSONObject(it)
            ConverterStatus(
                ok = if (status.has("ok")) status.getString("ok") else null,
                error = if (status.has("error")) status.getString("error") else null,
            )
        }
    }
    val lines = root.getJSONArray("lines").let { array ->
        List(array.length()) {
            if (array.isNull(it)) {
                null
            } else {
                val cell = array.getJSONObject(it)
                Cell(
                    text = cell.getString("text"),
                    copy = cell.getString("copy"),
                    error = if (cell.isNull("error")) null else cell.getString("error"),
                    total = cell.getBoolean("total"),
                )
            }
        }
    }
    return Calculated(converters, lines)
}
