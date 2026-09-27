package net.sybyline.scarlet.ext;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.util.Objects;

import net.sybyline.scarlet.ext.AvatarSearch.VrcxAvatar;
import net.sybyline.scarlet.util.HttpURLInputStream;
import net.sybyline.scarlet.util.URLs;

public interface AvatarSearch_AvatarSearch_CC
{
    static VrcxAvatar[] searchNamePC(String name)
    {
        return list("https://avatarsearch.cc/Avatar/NewAvatarSearcher?name="+URLs.encode(name));
    }
    static VrcxAvatar[] searchNameQuest(String name)
    {
        return list("https://avatarsearch.cc/Avatar/NewQuestAvatarSearcher?name="+URLs.encode(name));
    }
    static VrcxAvatar[] searchAuthorPC(String name)
    {
        return list("https://avatarsearch.cc/Avatar/NewAuthorSearcher?authorName="+URLs.encode(name));
    }
    static VrcxAvatar[] searchAuthorQuest(String name)
    {
        return list("https://avatarsearch.cc/Avatar/NewQuestAuthorSearcher?authorName="+URLs.encode(name));
    }
    static VrcxAvatar[] listRecentlyLogged()
    {
        return list("https://avatarsearch.cc/Avatar/RecentAvatars");
    }
    static VrcxAvatar[] listRandomPC()
    {
        return list("https://avatarsearch.cc/Avatar/RandomAvatarSearcher");
    }
    static VrcxAvatar[] listRandomQuest()
    {
        return list("https://avatarsearch.cc/Avatar/RandomQuestAvatarSearcher");
    }
    static VrcxAvatar[] list(String url)
    {
        try (BufferedReader in = new BufferedReader(new InputStreamReader(HttpURLInputStream.get(url, ExtendedUserAgent.init_conn))))
        {
            // Skip malformed/blank lines rather than letting one bad row throw and discard the whole result set.
            return in.lines().map(AvatarSearch_AvatarSearch_CC::parse).filter(Objects::nonNull).toArray(VrcxAvatar[]::new);
        }
        catch (Exception ex)
        {
            ex.printStackTrace();
            return null;
        }
    }
    static VrcxAvatar parse(String line)
    {
        if (line == null)
            return null;
        String[] linea = line.split("\\|", 4);
        if (linea.length < 2 || linea[0].isEmpty())
            return null;
        VrcxAvatar ret = new VrcxAvatar();
        ret.id          = linea[0];
        ret.name        = linea[1];
        ret.authorName  = linea.length > 2 ? linea[2] : "";
        ret.description = linea.length > 3 ? linea[3] : "";
        return ret;
    }
}
