package net.sybyline.scarlet.ext;

import java.io.File;
import java.net.URI;
import java.nio.charset.StandardCharsets;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.sun.jna.platform.win32.Advapi32;
import com.sun.jna.platform.win32.WinError;
import com.sun.jna.platform.win32.WinReg;
import com.sun.jna.ptr.IntByReference;

import net.sybyline.scarlet.util.Location;
import net.sybyline.scarlet.util.MiscUtils;
import net.sybyline.scarlet.util.Platform;
import net.sybyline.scarlet.util.Sys;
import net.sybyline.scarlet.util.URLs;

public interface VrcLaunch
{

    Logger LOG = LoggerFactory.getLogger("Scarlet/VrcLaunch");

    // vrchat://launch?ref=<Organization>&id=<Location>&shortName=<ShortName|SecureName>&attach=<integer>

    enum LaunchMode
    {
        VR(false),
        DESKTOP(true);

        LaunchMode(boolean noVr)
        {
            this.noVr = noVr;
        }
        final boolean noVr;
    }

    static void launch(String userId, Location location) throws Exception
    {
        launch(userId, location.world+':'+location.instance);
    }
    static void launch(String userId, Location location, boolean noVr) throws Exception
    {
        launch(userId, location.world+':'+location.instance, noVr);
    }
    static void launch(String userId, Location location, LaunchMode mode) throws Exception
    {
        launch(userId, location.world+':'+location.instance, mode);
    }
    static void launch(String userId, String location) throws Exception
    {
        launch(userId, location, LaunchMode.DESKTOP);
    }
    static void launch(String userId, String location, boolean noVr) throws Exception
    {
        launch(userId, location, noVr ? LaunchMode.DESKTOP : LaunchMode.VR);
    }
    static void launch(String userId, String location, LaunchMode mode) throws Exception
    {
        launch(userId, location, null, mode);
    }
    /**
     * Launches VRChat into a specific instance.
     *
     * <p>{@code shortName} is the instance's short/secure name from the VRChat API.
     * It is <b>required</b> to deep-link into any non-public instance (group,
     * group+, friends, invite, private): without it VRChat cannot resolve or
     * authorize the join and drops the client into the error/loading world even
     * though the instance exists. Public instances resolve from the location
     * alone, so a null shortName is fine for those.
     */
    static void launch(String userId, String location, String shortName, LaunchMode mode) throws Exception
    {
        // If a VRChat client is already running we must not cold-start a second one.
        // To "follow" into the new instance we quit the running client and then cold
        // launch into the target. A cold launch boots straight into the instance with
        // no prompt, so this is fully automatic and needs no launch flags. VRChat gives
        // external tools no way to move a running client in place without an in-game
        // prompt, so quit-and-relaunch is the only prompt-free path when one is already
        // up; when no client is running, the cold launch below simply joins directly.
        if (location != null && isVrChatRunning())
        {
            LOG.info("VRChat already running; quitting it to follow into {} with a single client", location);
            quitVrChat();
        }
        if (Platform.CURRENT.isNT())
            launch_win(userId, location, shortName, mode);
        else
            launch_linux(userId, location, shortName, mode);
    }

    /** Appends the VRChat deep-link query for a location, including {@code &shortName=} when present. */
    static String buildLaunchUri(String location, String shortName)
    {
        return buildLaunchUri(location, shortName, false);
    }

    /**
     * Builds the VRChat deep-link URI.
     *
     * @param attach when true, appends {@code &attach=1}, asking VRChat to handle
     *        the join in the <b>already-running</b> client (switch instance in
     *        place) rather than spinning up a new one. Only meaningful when a client
     *        is already running and the URI is fired through the OS protocol handler.
     */
    static String buildLaunchUri(String location, String shortName, boolean attach)
    {
        if (location == null)
            return null;
        StringBuilder uri = new StringBuilder("vrchat://launch?ref=KozyBlakeScarlet&id=").append(location);
        if (shortName != null && !shortName.trim().isEmpty())
            uri.append("&shortName=").append(URLs.encode(shortName.trim()));
        if (attach)
            uri.append("&attach=1");
        return uri.toString();
    }

    /** Whether a VRChat client is currently running. */
    static boolean isVrChatRunning()
    {
        return !findVrChatPids().isEmpty();
    }

    /**
     * PIDs of running VRChat clients (empty if none / detection failed). Windows asks
     * {@code tasklist} for VRChat.exe; other platforms use {@code pgrep} (VRChat runs
     * under Proton/Wine as VRChat.exe). Detection failure yields an empty list, so it
     * can only ever fall back to a normal cold launch, never block one.
     */
    static java.util.List<String> findVrChatPids()
    {
        java.util.List<String> pids = new java.util.ArrayList<>();
        try
        {
            ProcessBuilder pb = Platform.CURRENT.isNT()
                ? new ProcessBuilder("tasklist", "/FI", "IMAGENAME eq VRChat.exe", "/FO", "CSV", "/NH")
                : new ProcessBuilder("pgrep", "-f", "VRChat.exe");
            Process proc = pb.redirectErrorStream(true).start();
            try (java.io.BufferedReader reader = new java.io.BufferedReader(new java.io.InputStreamReader(proc.getInputStream(), StandardCharsets.UTF_8)))
            {
                String line;
                while ((line = reader.readLine()) != null)
                {
                    line = line.trim();
                    if (Platform.CURRENT.isNT())
                    {
                        if (!line.startsWith("\"VRChat.exe\""))
                            continue;
                        String[] cols = line.split("\",\"");
                        if (cols.length > 1)
                        {
                            String pid = cols[1].replace("\"", "").trim();
                            if (pid.matches("\\d+"))
                                pids.add(pid);
                        }
                    }
                    else if (line.matches("\\d+"))
                    {
                        pids.add(line);
                    }
                }
            }
            proc.waitFor(5L, java.util.concurrent.TimeUnit.SECONDS);
        }
        catch (Exception ex)
        {
            LOG.warn("VRChat pid lookup failed: {}", ex.getMessage());
        }
        return pids;
    }

    /**
     * Quits any running VRChat client - gracefully first, forcibly if it will not
     * exit - so a following cold launch lands in the new instance with exactly one
     * client and no in-game prompt.
     */
    static void quitVrChat()
    {
        if (findVrChatPids().isEmpty())
            return;
        try
        {
            if (Platform.CURRENT.isNT())
                new ProcessBuilder("taskkill", "/IM", "VRChat.exe").redirectErrorStream(true).start().waitFor(5L, java.util.concurrent.TimeUnit.SECONDS);
            else
            {
                java.util.List<String> cmd = new java.util.ArrayList<>();
                cmd.add("kill"); cmd.add("-TERM"); cmd.addAll(findVrChatPids());
                new ProcessBuilder(cmd).redirectErrorStream(true).start().waitFor(5L, java.util.concurrent.TimeUnit.SECONDS);
            }
            for (int i = 0; i < 20 && !findVrChatPids().isEmpty(); i++)
                Thread.sleep(500L);
            java.util.List<String> survivors = findVrChatPids();
            if (!survivors.isEmpty())
            {
                LOG.warn("VRChat did not exit gracefully; forcing {}", survivors);
                if (Platform.CURRENT.isNT())
                    new ProcessBuilder("taskkill", "/F", "/IM", "VRChat.exe").redirectErrorStream(true).start().waitFor(5L, java.util.concurrent.TimeUnit.SECONDS);
                else
                {
                    java.util.List<String> cmd = new java.util.ArrayList<>();
                    cmd.add("kill"); cmd.add("-KILL"); cmd.addAll(survivors);
                    new ProcessBuilder(cmd).redirectErrorStream(true).start().waitFor(5L, java.util.concurrent.TimeUnit.SECONDS);
                }
            }
        }
        catch (Exception ex)
        {
            LOG.warn("Failed to quit running VRChat: {}", ex.getMessage());
        }
    }

    /**
     * Launch VRChat on Linux via Steam.
     *
     * Strategy (each step falls back to the next on failure):
     *   1. steam -applaunch 438100 <args>   — most reliable, passes args directly
     *   2. steam steam://rungameid/438100    — works when steam binary is present but arg passing is unreliable
     *   3. xdg-open vrchat://...            — original behaviour, relies on URI handler registration
     */
    static void launch_linux(String userId, String location, String shortName, LaunchMode mode) throws Exception
    {
        if (mode == null)
            mode = LaunchMode.DESKTOP;
        String vrchatUri = buildLaunchUri(location, shortName);

        // Build the VRChat launch arguments (mirrors what launch_win passes on Windows)
        java.util.List<String> vrcArgs = new java.util.ArrayList<>();
        if (mode.noVr) { vrcArgs.add("--no-vr"); }
        vrcArgs.add("--enable-sdk-log-levels");
        vrcArgs.add("--enable-udon-debug-logging");
        vrcArgs.add("--enable-verbose-logging");
        vrcArgs.add("--log-debug-levels=API;All;Always;AssetBundleDownloadManager;ContentCreator;Errors;NetworkData;NetworkProcessing;NetworkTransport;Warnings");
        if (location != null) { vrcArgs.add(vrchatUri); }
        if (userId   != null) { vrcArgs.add("--profile=" + userId); }

        // Strategy 1: steam -applaunch
        if (Sys.hasInPath("steam"))
        {
            try
            {
                java.util.List<String> cmd = new java.util.ArrayList<>();
                cmd.add("steam");
                cmd.add("-applaunch");
                cmd.add(VrcAppData.VRCHAT_APP_ID);
                cmd.addAll(vrcArgs);
                LOG.info("Launching VRChat via: {}", cmd);
                new ProcessBuilder(cmd).inheritIO().start();
                return;
            }
            catch (Exception ex)
            {
                LOG.warn("steam -applaunch failed, trying steam:// URI: {}", ex.getMessage());
            }

            // Strategy 2: steam steam://rungameid/438100//<args>
            // Steam accepts launch arguments appended after a "//" separator; without
            // them this would open VRChat with no instance and land on the home world.
            try
            {
                StringBuilder runGameId = new StringBuilder("steam://rungameid/").append(VrcAppData.VRCHAT_APP_ID);
                if (!vrcArgs.isEmpty())
                {
                    runGameId.append("//");
                    for (int i = 0; i < vrcArgs.size(); i++)
                    {
                        if (i > 0)
                            runGameId.append(' ');
                        runGameId.append(vrcArgs.get(i));
                    }
                }
                LOG.info("Launching VRChat via {}", runGameId);
                new ProcessBuilder("steam", runGameId.toString())
                    .inheritIO().start();
                return;
            }
            catch (Exception ex)
            {
                LOG.warn("steam rungameid failed, falling back to xdg-open URI: {}", ex.getMessage());
            }
        }
        else
        {
            LOG.warn("steam binary not found in PATH, falling back to URI handler");
        }

        // Strategy 3: original xdg-open / URI handler fallback
        if (vrchatUri != null)
        {
            LOG.info("Launching VRChat via URI handler: {}", vrchatUri);
            MiscUtils.AWTDesktop.browse(URI.create(vrchatUri));
        }
    }
    static void launch_win(String userId, String location, String shortName, LaunchMode mode) throws Exception
    {
        if (mode == null)
            mode = LaunchMode.DESKTOP;
        String path;
        {
            IntByReference pType = new IntByReference(),
                    pcbData = new IntByReference(256);
             byte[] buffer = new byte[256];
             int werr = Advapi32.INSTANCE.RegGetValue(WinReg.HKEY_CURRENT_USER, "Software\\VRChat", "", Advapi32.RRF_RT_REG_SZ, pType, buffer, pcbData);
             if (werr == WinError.ERROR_MORE_DATA)
             {
                 buffer = new byte[pcbData.getValue()];
                 werr = Advapi32.INSTANCE.RegGetValue(WinReg.HKEY_CURRENT_USER, "Software\\VRChat", "", Advapi32.RRF_RT_REG_SZ, pType, buffer, pcbData);
             }
             if (werr != WinError.ERROR_SUCCESS)
                 throw new Exception(String.format("Failed to locate VRChat via registry: 0x%08x", werr));
             int regLen = pcbData.getValue() - 2;
             if (regLen <= 0)
                 throw new Exception("Failed to locate VRChat via registry: empty value");
             path = new String(buffer, 0, regLen, StandardCharsets.UTF_16LE);
        }
        java.util.List<String> cmd = new java.util.ArrayList<>();
        cmd.add(new File(path, "launch.exe").getAbsolutePath());

        String vrchatUri = buildLaunchUri(location, shortName);
        if (vrchatUri != null)
        {
            cmd.add(vrchatUri);
        }

        if (userId != null)
        {
            cmd.add("--profile=" + userId);
        }

        if (mode.noVr)
        {
            cmd.add("--no-vr");
        }

        cmd.add("--enable-sdk-log-levels");
        cmd.add("--enable-udon-debug-logging");
        cmd.add("--enable-verbose-logging");
        cmd.add("--log-debug-levels=API;All;Always;AssetBundleDownloadManager;ContentCreator;Errors;NetworkData;NetworkProcessing;NetworkTransport;Warnings");

        LOG.info("Cold-launching VRChat: uri={} cmd={}", vrchatUri, cmd);
        new ProcessBuilder(cmd).start();
    }

}
