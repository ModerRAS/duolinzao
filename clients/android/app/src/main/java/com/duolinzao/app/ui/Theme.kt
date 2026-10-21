package com.duolinzao.app.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp

private val Green = Color(0xFF2D8A45)

@Composable
fun DlTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = lightColorScheme(primary = Green, secondary = Green),
        content = content,
    )
}

@Composable
fun DlTextField(
    value: String,
    onValue: (String) -> Unit,
    label: String,
    password: Boolean = false,
    modifier: Modifier = Modifier,
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValue,
        label = { Text(label) },
        singleLine = true,
        visualTransformation = if (password) PasswordVisualTransformation() else VisualTransformation.None,
        modifier = modifier.fillMaxWidth(),
    )
}

@Composable
fun DlPrimaryButton(text: String, enabled: Boolean = true, onClick: () -> Unit) {
    Button(
        onClick = onClick,
        enabled = enabled,
        colors = ButtonDefaults.buttonColors(containerColor = Green),
        modifier = Modifier.fillMaxWidth(),
    ) { Text(text) }
}

@Composable
fun DlOutlineButton(text: String, onClick: () -> Unit) {
    OutlinedButton(onClick = onClick, modifier = Modifier.fillMaxWidth()) { Text(text) }
}

@Composable
fun ErrorLine(message: String?) {
    if (!message.isNullOrEmpty()) {
        Spacer(Modifier.height(8.dp))
        Text(message, color = Color(0xFFE53935), style = MaterialTheme.typography.bodySmall)
    }
}

/** 成功提示走绿色，别再用红色 ErrorLine 显示「已提交/已保存」。 */
@Composable
fun SuccessLine(message: String?) {
    if (!message.isNullOrEmpty()) {
        Spacer(Modifier.height(8.dp))
        Text(message, color = Color(0xFF2D8A45), style = MaterialTheme.typography.bodySmall)
    }
}

@Composable
fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(vertical = 8.dp))
}

/** 报餐色：绿=报，橙=不报（与 Web 预览一致）；locked 灰。 */
fun mealColor(color: String): Color = when (color) {
    "green" -> Color(0xFF2D8A45)
    "orange" -> Color(0xFFE6A23C)
    "locked" -> Color(0xFF9E9E9E)
    else -> Color(0xFFD1D5DB)
}

fun slotName(slot: String): String = when (slot) {
    "breakfast" -> "早餐"
    "lunch" -> "午餐"
    "dinner" -> "晚餐"
    else -> slot
}
