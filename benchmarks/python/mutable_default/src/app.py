def append_item(item, bucket=[]):
    bucket.append(item)
    return bucket


def keyed(items, lookup={}):
    lookup.update(items)
    return lookup


def safe_append(item, bucket=None):
    if bucket is None:
        bucket = []
    bucket.append(item)
    return bucket
