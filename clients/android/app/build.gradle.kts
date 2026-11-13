plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

android {
    namespace = "com.duolinzao.app"
    compileSdk = 35

    defaultConfig {
        // 标识符 duolinzao。CI 经 -PdlzVersionCode/-PdlzVersionName 传入
        // Gitea 风格串（scripts/ci-version.sh）与 git rev-list 计数；
        // 本地不传时保留默认值，本地构建不破。
        // 通用 APK 的 versionCode/versionName 由 inject 脚本原样读取，部署方不覆盖。
        applicationId = "com.duolinzao.app"
        minSdk = 30
        targetSdk = 35
        val dlzVersionCode = (findProperty("dlzVersionCode") as String?)?.toIntOrNull() ?: 1
        val dlzVersionName = (findProperty("dlzVersionName") as String?)?.trim()?.takeIf { it.isNotEmpty() } ?: "0.1.2"
        versionCode = dlzVersionCode
        versionName = dlzVersionName
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        manifestPlaceholders["cleartextPermitted"] = false
    }

    buildTypes {
        release {
            isMinifyEnabled = false
        }
        debug {
            // 仅 debug 允许 http 明文（模拟器连宿主机后端 10.0.2.2 用）。
            manifestPlaceholders["cleartextPermitted"] = true
        }
    }
    buildFeatures {
        compose = true
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

dependencies {
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)
    implementation(libs.compose.material3)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.runtime)
    implementation(libs.lifecycle.viewmodel)
    implementation(libs.navigation.compose)
    implementation(libs.okhttp)
    implementation(libs.serialization.json)
    implementation(libs.coroutines.android)
    // Passkey：Credential Manager 平台 API，禁止 WebView。无 GMS/无凭据时自动回退密码。
    implementation(libs.credentials)
    implementation(libs.credentials.play.auth)
    // WebView 套壳：系统 WebAuthn/Passkey（Credential Manager）+ WebSettingsCompat。
    implementation(libs.webkit)
    // 壳 BACK 状态机等纯 Kotlin 逻辑的 JVM 单测（app/src/test，不依赖 Android 运行时）。
    testImplementation(libs.junit.jvm)
    // Compose UI 冒烟测试（mock，不连真后端，见 SmokeUiTest）。
    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.test.junit)
    androidTestImplementation(libs.espresso.core)
    debugImplementation(libs.compose.ui.test.manifest)
}
