package io.github.marelevn.soos

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * The desktop app's palette (soos-app's `style.rs`), colour for colour. Every
 * text colour has at least 4.5:1 contrast with `background`. Colour is for
 * the answers: what's typed is neutral in three steps -- `plain`, then
 * `secondary` for labels and joining words, then `comment` -- with one accent
 * for the words that stand for a value (`sum`, `prev`, `today`).
 */
data class Palette(
    val background: Color,
    val plain: Color,
    val result: Color,
    val error: Color,
    val keyword: Color,
    val secondary: Color,
    val comment: Color,
)

val DarkPalette = Palette(
    background = Color(0xFF222428),
    plain = Color(0xFFF0F0F0),
    result = Color(0xFF96C85A),
    error = Color(0xFFD86C6C),
    keyword = Color(0xFF5BC8E8),
    secondary = Color(0xFFB8BCC3),
    comment = Color(0xFF959AA2),
)

val LightPalette = Palette(
    background = Color(0xFFF7F8FA),
    plain = Color(0xFF14161A),
    result = Color(0xFF387F2A),
    error = Color(0xFFC02020),
    keyword = Color(0xFF0B7A9E),
    secondary = Color(0xFF4A4F57),
    comment = Color(0xFF626973),
)

@Composable
fun palette(): Palette = if (isSystemInDarkTheme()) DarkPalette else LightPalette

/** The font size every surface uses, in sp so the system's font scale applies. */
val FontSize = 15.sp

/** JetBrains Mono is 600/1000 em wide, so a width for N characters is N times this. */
const val CharWidthEm = 0.6f

/** Regular and bold, read from `assets/fonts` (the build adds that folder as is). */
@Composable
fun mono(): FontFamily {
    val assets = LocalContext.current.assets
    return remember {
        FontFamily(
            Font("JetBrainsMono-Regular.ttf", assets, FontWeight.Normal),
            Font("JetBrainsMono-Bold.ttf", assets, FontWeight.Bold),
        )
    }
}
