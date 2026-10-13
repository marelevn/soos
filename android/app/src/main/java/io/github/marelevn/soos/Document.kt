package io.github.marelevn.soos

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay

/** Shown in an empty document. */
const val PLACEHOLDER = "Type a calculation, like 15% off \$120"

/** Once results have been out of date this long, they're drawn dimmed. */
private const val STALE_AFTER_MS = 150L

/** How far out-of-date results fade toward the background. */
private const val STALE_DIM = 0.55f

/**
 * An error on the line being typed waits this long after the last keystroke:
 * half a line is usually an error (`5 k` before `5 kg`).
 */
private const val ERROR_HOLD_MS = 900L

private val PagePad = 16.dp

/** Between the expressions and the result column. */
private val ColumnGap = 12.dp
private val GutterPad = 4.dp
private val MinGutter = 72.dp

/** So a long result can't squeeze out the expressions. */
private const val MaxGutterFraction = 0.4f

/** The line height the desktop uses, 1.75 times the font size. */
private val LineHeight = FontSize * 1.75f

/** The result column's width: the widest value, within limits. Errors don't count; a long one is cut. */
private fun gutterWidth(cells: List<Cell?>, available: Dp, charWidth: Dp): Dp {
    val widest = cells.filterNotNull()
        .filter { it.error == null }
        .maxOfOrNull { it.text.codePointCount(0, it.text.length) } ?: 0
    return (charWidth * widest + GutterPad * 2).coerceAtLeast(MinGutter)
        .coerceAtMost(available * MaxGutterFraction)
}

/** Colours the spans soos-core finds; what's outside them is plain text. */
private class Highlighter(private val palette: Palette) : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText {
        val source = text.text
        val styled = buildAnnotatedString {
            append(source)
            for (span in Engine.spans(source)) {
                val start = span.start.coerceIn(0, source.length)
                val end = span.end.coerceIn(start, source.length)
                when (span.kind) {
                    TokenKind.Comment -> addStyle(SpanStyle(color = palette.comment), start, end)
                    // The `#` marks a heading; the heading itself is the
                    // strongest text on the page, not the faintest.
                    TokenKind.Header -> {
                        var marker = start
                        while (marker < end && (source[marker] == '#' || source[marker] == ' ')) marker++
                        addStyle(SpanStyle(color = palette.comment), start, marker)
                        addStyle(SpanStyle(fontWeight = FontWeight.Bold), marker, end)
                    }
                    TokenKind.Label, TokenKind.ConversionWord ->
                        addStyle(SpanStyle(color = palette.secondary), start, end)
                    TokenKind.Keyword -> addStyle(SpanStyle(color = palette.keyword), start, end)
                }
            }
        }
        return TransformedText(styled, OffsetMapping.Identity)
    }
}

/**
 * The active tab: one text field, so selection, undo and the caret work as
 * usual, and each line's result in a column beside it. A wrapped line's
 * result sits beside its first row.
 */
@Composable
fun Document(state: AppState, palette: Palette, modifier: Modifier = Modifier) {
    val tab = state.activeTab
    val font = mono()
    val cells = state.calculated?.lines ?: emptyList()

    val stale = rememberStale(state)
    val typing = rememberTyping(state)

    val style = remember(palette, font) {
        TextStyle(
            fontFamily = font,
            fontSize = FontSize,
            lineHeight = LineHeight,
            color = palette.plain,
            // Centred in the line, so the text sits level with its result.
            lineHeightStyle = LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None),
        )
    }
    val highlighter = remember(palette) { Highlighter(palette) }
    val focus = remember { FocusRequester() }
    // A new or switched-to tab opens with the keyboard up and the caret
    // where it was (at the end, in a tab that was just loaded).
    LaunchedEffect(tab.id) { focus.requestFocus() }

    val charWidth = with(LocalDensity.current) { (FontSize * CharWidthEm).toDp() }
    val text = tab.text
    val lineStarts = remember(text) {
        buildList {
            add(0)
            text.forEachIndexed { i, c -> if (c == '\n') add(i + 1) }
        }
    }
    var layout by remember(tab.id) { mutableStateOf<TextLayoutResult?>(null) }
    val caretLine = lineStarts.indexOfLast { it <= tab.value.selection.end }

    BoxWithConstraints(modifier.fillMaxSize()) {
        val gutter = gutterWidth(cells, maxWidth - PagePad * 2, charWidth)
        // Read here: inside the nested Box this scope's size isn't reachable.
        val viewport = maxHeight
        Box(
            Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = PagePad, vertical = 12.dp),
        ) {
            BasicTextField(
                value = tab.value,
                onValueChange = { state.edit(tab, it) },
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(end = gutter + ColumnGap)
                    // A tap below the text still places the caret.
                    .defaultMinSize(minHeight = viewport - 24.dp)
                    .focusRequester(focus),
                textStyle = style,
                cursorBrush = SolidColor(palette.plain),
                visualTransformation = highlighter,
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.None,
                    autoCorrectEnabled = false,
                ),
                onTextLayout = { layout = it },
                decorationBox = { field ->
                    Box {
                        if (text.isEmpty()) {
                            BasicText(PLACEHOLDER, style = style.copy(color = palette.comment))
                        }
                        field()
                    }
                },
            )

            val measured = layout
            if (measured != null) {
                cells.forEachIndexed { line, cell ->
                    if (cell == null || line >= lineStarts.size) return@forEachIndexed
                    val isError = cell.error != null
                    if (isError && typing && line == caretLine) return@forEachIndexed
                    val offset = lineStarts[line].coerceAtMost(measured.layoutInput.text.length)
                    val top = measured.getLineTop(measured.getLineForOffset(offset))
                    val color = if (isError) palette.error else palette.result
                    BasicText(
                        text = cell.text,
                        style = style.copy(
                            color = if (stale) lerp(color, palette.background, STALE_DIM) else color,
                            fontWeight = if (cell.total && !isError) FontWeight.Bold else FontWeight.Normal,
                            textAlign = TextAlign.End,
                        ),
                        maxLines = 1,
                        softWrap = false,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier
                            .align(Alignment.TopEnd)
                            .offset { IntOffset(0, top.toInt()) }
                            .width(gutter)
                            // A value copies; an error says what's wrong, in the status bar.
                            .clickable(
                                role = Role.Button,
                                onClickLabel = if (isError) "Show the error" else "Copy the result",
                            ) {
                                if (cell.error != null) {
                                    state.show(cell.error, error = true, forMillis = 4000)
                                } else {
                                    state.copy(cell)
                                }
                            },
                    )
                }
            }
        }
    }
}

/** Whether results have been out of date for [STALE_AFTER_MS], so they're drawn dimmed. */
@Composable
private fun rememberStale(state: AppState): Boolean {
    val pending = state.requested != state.received
    var slow by remember { mutableStateOf(false) }
    LaunchedEffect(pending, state.requested) {
        slow = false
        if (pending) {
            delay(STALE_AFTER_MS)
            slow = true
        }
    }
    return pending && slow
}

/** True for [ERROR_HOLD_MS] after each edit. */
@Composable
private fun rememberTyping(state: AppState): Boolean {
    var typing by remember { mutableStateOf(false) }
    LaunchedEffect(state.edits) {
        if (state.edits > 0) {
            typing = true
            delay(ERROR_HOLD_MS)
            typing = false
        }
    }
    return typing
}
