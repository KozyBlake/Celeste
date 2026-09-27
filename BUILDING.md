# Building Scarlet from source

## Requirements

| | |
|---|---|
| JDK | **21** (bytecode targets Java 8) |
| Maven | 3.9 or newer |
| Network | required — most dependencies come from Maven Central |

Verified against OpenJDK 21.0.12 and Apache Maven 3.9.16 on Linux.

No native toolchain is needed for a normal desktop build. The DAVE native
libraries for Linux, Windows and macOS are pulled from Maven Central as
prebuilt artifacts (`moe.kyokobot.libdave:natives-*:0.1.2`).

## Build

**The order matters.** `libdave-maven/` must be installed into your local
Maven repository before the root project will resolve.

```sh
git clone <your-fork-url> Scarlet
cd Scarlet

# 1. build and install the vendored libdave-jvm modules
cd libdave-maven
mvn -B clean install
cd ..

# 2. build Scarlet
mvn -B clean package
```

Roughly 20-30 seconds for the root project on a modern machine.

## Why libdave has to be built first

Scarlet depends on four artifacts at version `1.0.0`:

- `moe.kyokobot.libdave:libdave-jvm-api`
- `moe.kyokobot.libdave:libdave-jvm-impl`
- `moe.kyokobot.libdave:libdave-jvm-adapter-jda`
- `moe.kyokobot.libdave:libdave-jvm-natives-android`

**These are not published to any public Maven repository.** Their sources are
vendored in `libdave-maven/`, and `mvn install` there places them in your
local `~/.m2/repository` where the root build can find them.

Skip that step and the build fails like this:

```
[ERROR] Failed to read artifact descriptor for
        moe.kyokobot.libdave:libdave-jvm-adapter-jda:jar:1.0.0
[ERROR]   Caused by: ... could not be resolved: ... from/to jitpack.io
          (https://jitpack.io): status code: 401, reason phrase: Unauthorized (401)
```

**JitPack is not the problem here.** It is simply the last repository in the
resolution order, and it answers `401 Unauthorized` — rather than `404` — for
any artifact it does not host. So it takes the blame for an artifact that was
never meant to come from the network at all. No JitPack account or token is
required to build Scarlet.

## Build outputs

All in `target/`:

| Artifact | Purpose |
|---|---|
| `scarlet-<version>.jar` | **the one you want** — shaded, desktop natives only |
| `scarlet-<version>-debug.jar` | same, with debug logging enabled |
| `scarlet-<version>-android.jar` | Android/Termux natives instead of desktop ones |
| `original-scarlet-<version>.jar` | pre-shade jar; not runnable on its own |

Run with:

```sh
java -jar target/scarlet-<version>.jar
```

## Optional: compiling Android DAVE natives yourself

The `-android` jar ships prebuilt `.so` files by default, so this is only
needed if you want to rebuild them from source. Requires the Android NDK:

```sh
export ANDROID_NDK_HOME=/path/to/android-ndk
mvn -B clean package -Pandroid-dave-native
```

A `termux-local-dave` profile also exists for supplying an
already-compiled native via `-Dscarlet.localAndroidDaveNative=/path/to/libdave-jvm.so`.

## Troubleshooting

**`401 Unauthorized` from jitpack.io** — you skipped the `libdave-maven`
install. See above.

**Still failing after installing libdave** — Maven caches resolution failures
and will not retry until they expire. Clear them:

```sh
find ~/.m2/repository/moe/kyokobot/libdave -name "*.lastUpdated" -delete
```

**Verify libdave installed correctly:**

```sh
ls ~/.m2/repository/moe/kyokobot/libdave/*/1.0.0/*.jar
```

You should see four jars. If the directories contain only `.lastUpdated`
files, the install did not happen.
