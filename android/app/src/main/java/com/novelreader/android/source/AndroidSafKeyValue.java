package com.novelreader.android.source;

import android.content.Context;
import android.content.SharedPreferences;

import java.util.HashSet;
import java.util.Map;
import java.util.Set;

/**
 * SAF 来源记录的偏好存储实现（生产侧 {@link SafKeyValueStore}）。
 *
 * <p>键即 SAF URI（{@code source:content://…}）：URI 里没有换行，SharedPreferences 的 XML
 * 转义足以承载。单个键的值是 {@code SafSourceCodec} 的文本格式。
 *
 * <p>只保存 application context，且不做任何「整本正文」相关的存取：这里存的永远是元数据。
 */
public final class AndroidSafKeyValue implements SafKeyValueStore {

    private static final String PREFERENCES_NAME = "novel-reader-saf-sources";

    private final SharedPreferences preferences;

    public AndroidSafKeyValue(Context context) {
        Context application = context.getApplicationContext();
        Context target = application != null ? application : context;
        this.preferences = target.getSharedPreferences(PREFERENCES_NAME, Context.MODE_PRIVATE);
    }

    @Override
    public String get(String key) {
        return preferences.getString(key, null);
    }

    @Override
    public void put(String key, String value) {
        preferences.edit().putString(key, value).apply();
    }

    @Override
    public void remove(String key) {
        preferences.edit().remove(key).apply();
    }

    @Override
    public Set<String> keys() {
        return new HashSet<>(preferences.getAll().keySet());
    }

    /** 记录条数（诊断用）。 */
    public int size() {
        Map<String, ?> all = preferences.getAll();
        return all.size();
    }
}
