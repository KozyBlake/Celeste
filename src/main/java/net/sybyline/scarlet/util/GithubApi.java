package net.sybyline.scarlet.util;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;

public interface GithubApi
{

    static String[] release_names(String owner, String repo)
    {
        JsonArray ja;
        try (HttpURLInputStream in = HttpURLInputStream.get(String.format("https://api.github.com/repos/%s/%s/releases", owner, repo)))
        {
            ja = in.readAsJson(null, null, JsonArray.class);
        }
        catch (Exception ex)
        {
            return null;
        }
        return ja.asList().stream()
            .map(JsonElement::getAsJsonObject)
            .map($ -> $.has("tag_name") && !$.get("tag_name").isJsonNull() ? $.get("tag_name")
                    : $.has("name") && !$.get("name").isJsonNull() ? $.get("name") : null)
            .map($ -> $ == null ? null : $.getAsString())
            .filter(java.util.Objects::nonNull)
            .toArray(String[]::new);
    }

}
